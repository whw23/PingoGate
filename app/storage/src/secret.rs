//! Secret-reference resolution.
//!
//! This phase supports only `env:VAR_NAME` references (config contract). The
//! resolver reads the environment at snapshot-build time and returns a
//! [`SecretString`]; error messages name the variable, never its value.

use pingo_core::SecretString;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret reference is empty")]
    Empty,
    #[error("unsupported secret reference scheme: {0}")]
    UnsupportedScheme(String),
    #[error("environment variable not set: {0}")]
    MissingEnv(String),
    #[error("resolved secret is empty: {0}")]
    EmptyValue(String),
}

/// Resolves an opaque secret reference (e.g. `env:OPENAI_API_KEY`) to material.
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<SecretString, SecretError>;
}

/// Resolver backed by process environment variables.
#[derive(Debug, Default, Clone)]
pub struct EnvSecretResolver;

impl SecretResolver for EnvSecretResolver {
    fn resolve(&self, reference: &str) -> Result<SecretString, SecretError> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Err(SecretError::Empty);
        }
        let (scheme, rest) = reference
            .split_once(':')
            .ok_or_else(|| SecretError::UnsupportedScheme(reference.to_string()))?;
        match scheme {
            "env" => {
                let var = rest.trim();
                let value =
                    std::env::var(var).map_err(|_| SecretError::MissingEnv(var.to_string()))?;
                if value.is_empty() {
                    return Err(SecretError::EmptyValue(var.to_string()));
                }
                Ok(SecretString::new(value))
            }
            other => Err(SecretError::UnsupportedScheme(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_existing_env_var() {
        std::env::set_var("PINGO_TEST_SECRET_OK", "resolved-value");
        let r = EnvSecretResolver;
        let s = r.resolve("env:PINGO_TEST_SECRET_OK").unwrap();
        assert_eq!(s.expose(), "resolved-value");
    }

    #[test]
    fn missing_env_var_errors_without_leaking_value() {
        let r = EnvSecretResolver;
        let err = r
            .resolve("env:PINGO_TEST_DEFINITELY_UNSET_VAR")
            .unwrap_err();
        assert!(matches!(err, SecretError::MissingEnv(_)));
        assert!(err.to_string().contains("PINGO_TEST_DEFINITELY_UNSET_VAR"));
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        let r = EnvSecretResolver;
        assert!(matches!(
            r.resolve("vault:foo").unwrap_err(),
            SecretError::UnsupportedScheme(_)
        ));
        assert!(matches!(
            r.resolve("noscheme").unwrap_err(),
            SecretError::UnsupportedScheme(_)
        ));
    }

    #[test]
    fn empty_reference_is_rejected() {
        let r = EnvSecretResolver;
        assert!(matches!(r.resolve("   ").unwrap_err(), SecretError::Empty));
    }
}
