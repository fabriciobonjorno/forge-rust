//! Strict query-string parsing.
//!
//! The catalog's parameters are identifiers and integers, which never need
//! percent-encoding, so values are used verbatim and anything unexpected is
//! rejected instead of guessed: unknown or repeated parameters, empty
//! segments, and segments without `=` or without a value all fail with a
//! `Validation` error (code `invalid_query`, metadata `parameter`).

use std::collections::BTreeMap;

use forge::core::{ErrorCategory, ForgeError};

/// Parses `query` (the part after `?`, if any) into its parameters,
/// accepting only names listed in `allowed`, each at most once.
pub fn parse<'a>(
    query: Option<&'a str>,
    allowed: &[&str],
) -> Result<BTreeMap<&'a str, &'a str>, ForgeError> {
    let mut parameters = BTreeMap::new();
    let Some(query) = query.filter(|query| !query.is_empty()) else {
        return Ok(parameters);
    };
    for segment in query.split('&') {
        let Some((name, value)) = segment.split_once('=') else {
            return Err(invalid(
                "query parameters must be written as name=value",
                segment,
            ));
        };
        if !allowed.contains(&name) {
            return Err(invalid("unknown query parameter", name));
        }
        if value.is_empty() {
            return Err(invalid("query parameter must have a value", name));
        }
        if parameters.insert(name, value).is_some() {
            return Err(invalid("query parameter must not be repeated", name));
        }
    }
    Ok(parameters)
}

fn invalid(message: &'static str, parameter: &str) -> ForgeError {
    // Echo at most a short prefix of attacker-controlled input.
    let parameter: String = parameter.chars().take(64).collect();
    ForgeError::new(ErrorCategory::Validation, "invalid_query", message)
        .with_metadata("parameter", parameter)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &[&str] = &["id", "limit"];

    fn rejected(query: &str) -> Option<(String, String)> {
        parse(Some(query), ALLOWED).err().map(|error| {
            assert_eq!(error.category(), ErrorCategory::Validation);
            let parameter = error
                .metadata()
                .get("parameter")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_owned();
            (error.code().to_owned(), parameter)
        })
    }

    #[test]
    fn absent_or_empty_query_has_no_parameters() -> Result<(), ForgeError> {
        assert!(parse(None, ALLOWED)?.is_empty());
        assert!(parse(Some(""), ALLOWED)?.is_empty());
        Ok(())
    }

    #[test]
    fn allowed_parameters_are_returned_verbatim() -> Result<(), ForgeError> {
        let parameters = parse(Some("limit=5&id=abc"), ALLOWED)?;

        assert_eq!(parameters.get("id"), Some(&"abc"));
        assert_eq!(parameters.get("limit"), Some(&"5"));
        assert_eq!(parameters.len(), 2);
        Ok(())
    }

    #[test]
    fn malformed_unknown_and_duplicate_parameters_are_rejected() {
        let code = "invalid_query".to_owned();
        for (query, parameter) in [
            ("id=1&id=2", "id"),
            ("sort=name", "sort"),
            ("id", "id"),
            ("id=", "id"),
            ("id=1&", ""),
            ("&id=1", ""),
            ("=1", ""),
            ("ID=1", "ID"),
            ("id%3D=1", "id%3D"),
        ] {
            assert_eq!(
                rejected(query),
                Some((code.clone(), parameter.to_owned())),
                "{query:?}"
            );
        }
    }

    #[test]
    fn echoed_parameter_names_are_bounded() {
        let long = "x".repeat(1_000);
        let echoed = rejected(&format!("{long}=1")).map(|(_, parameter)| parameter.len());
        assert_eq!(echoed, Some(64));
    }
}
