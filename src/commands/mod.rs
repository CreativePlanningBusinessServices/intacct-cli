pub mod account;
pub mod composite;
pub mod config_cmd;
pub mod describe;
pub mod export;
pub mod job;
pub mod object;
pub mod query;
pub mod raw;
pub mod report;
pub mod skill;
pub mod update;
pub mod view;

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use crate::client::BinaryResponse;
use crate::error::CliError;

/// Shared by `export` and `report download`: both endpoints return a binary file body and
/// write it to `output_path`, refusing to clobber an existing file, with the same
/// `{"written", "bytes", "contentType"}` result shape.
pub fn write_binary_output(
    output_path: &Path,
    response: BinaryResponse,
) -> Result<Value, CliError> {
    std::fs::write(output_path, &response.bytes).map_err(|write_error| {
        CliError::Usage(format!(
            "cannot write {}: {write_error}",
            output_path.display()
        ))
    })?;
    Ok(json!({
        "written": output_path.display().to_string(),
        "bytes": response.bytes.len(),
        "contentType": response.content_type,
    }))
}

pub fn refuse_existing_output(output_path: &Path) -> Result<(), CliError> {
    if output_path.exists() {
        return Err(CliError::Usage(format!(
            "output file already exists: {}",
            output_path.display()
        )));
    }
    Ok(())
}

pub fn read_data_arg(raw: &str) -> Result<Value, CliError> {
    let text = if raw == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|io_error| CliError::Usage(format!("cannot read stdin: {io_error}")))?;
        buffer
    } else if let Some(file_path) = raw.strip_prefix('@') {
        std::fs::read_to_string(file_path)
            .map_err(|io_error| CliError::Usage(format!("cannot read {file_path}: {io_error}")))?
    } else {
        raw.to_string()
    };
    serde_json::from_str(&text)
        .map_err(|parse_error| CliError::Usage(format!("--data is not valid JSON: {parse_error}")))
}

/// Parses repeated `--param`/`--query key=value` flags; used by the raw command.
pub fn parse_key_value_pairs(
    pairs: &[String],
    flag_name: &str,
) -> Result<Vec<(String, String)>, CliError> {
    pairs
        .iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or_else(|| {
                CliError::Usage(format!(
                    "{flag_name} must be in key=value form, got '{pair}'"
                ))
            })?;
            Ok((key.to_string(), value.to_string()))
        })
        .collect()
}

/// Parses repeated `--header 'Name: value'` flags for the raw command.
pub fn parse_header_pairs(pairs: &[String]) -> Result<Vec<(String, String)>, CliError> {
    pairs
        .iter()
        .map(|pair| {
            let (name, value) = pair.split_once(':').ok_or_else(|| {
                CliError::Usage(format!(
                    "--header must be in 'Name: value' form, got '{pair}'"
                ))
            })?;
            Ok((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn reads_inline_json() {
        let parsed = read_data_arg(r#"{"companyName": "Acme"}"#).unwrap();
        assert_eq!(parsed, serde_json::json!({"companyName": "Acme"}));
    }

    #[test]
    fn reads_json_from_file_argument() {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        write!(temp_file, r#"{{"companyName": "Acme"}}"#).unwrap();
        let file_arg = format!("@{}", temp_file.path().display());

        let parsed = read_data_arg(&file_arg).unwrap();
        assert_eq!(parsed, serde_json::json!({"companyName": "Acme"}));
    }

    #[test]
    fn rejects_invalid_json() {
        let parse_result = read_data_arg("not json");
        assert!(matches!(parse_result, Err(CliError::Usage(_))));
    }

    #[test]
    fn rejects_missing_file() {
        let parse_result = read_data_arg("@/nonexistent");
        assert!(matches!(parse_result, Err(CliError::Usage(_))));
    }
}
