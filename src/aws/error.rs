//! One error type for everything that talks to AWS, shaped for the person reading it.
//!
//! AWS has hundreds of error codes, and each service spells the same idea differently: SSM says
//! `AccessDeniedException`, S3 says `AccessDenied`, the SSO portal answers a bare 403 with a JSON
//! `message`. Nobody unlocking a vault cares which spelling came back. They care about a short list
//! of things they can act on: *you are not allowed*, *it is not there*, *log in again*, *slow
//! down*, *the network is down*, *AWS rejected what was sent*. So every response is folded into
//! one of those here, once, and the interface turns each variant into one sentence in Portuguese.
//!
//! The original code and message are kept wherever AWS gave one, because the sentence the
//! interface shows is for the person and the code is for whoever has to debug it later.
//!
//! **No error ever carries a secret.** Messages come from AWS's response bodies, which echo
//! parameter *names* and ARNs but never values or credentials, and network errors are stripped of
//! their URL before they are kept.

/// Everything that can go wrong talking to AWS, in the terms the interface explains.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AwsError {
    /// The credentials are valid, but the role or policy does not allow this action (or the KMS
    /// key behind a SecureString refuses to decrypt for this principal).
    #[error("access denied: {0}")]
    AccessDenied(String),

    /// The parameter, version, bucket or object does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// The write lost a race or would have overwritten something it was told not to: a
    /// parameter that already exists when `Overwrite` was false, or an S3 precondition that failed.
    #[error("conflict: {0}")]
    VersionConflict(String),

    /// The token or the temporary credentials are no longer accepted. The only fix is to log in
    /// again (or refresh, when a refresh token is still good).
    #[error("session expired: {0}")]
    SessionExpired(String),

    /// AWS asked to slow down. Retrying later, not immediately, is the answer.
    #[error("throttled by AWS: {0}")]
    Throttled(String),

    /// Nothing usable came back: DNS, TLS, a refused connection, a timeout, or a body that was
    /// cut off halfway.
    #[error("network error: {0}")]
    Network(String),

    /// AWS (or this module, before sending) rejected the request as malformed: a bad name, a
    /// value too long, an unknown KMS key, a bucket in another region.
    #[error("invalid request: {0}")]
    Invalid(String),

    /// Anything else, kept verbatim so it can at least be reported.
    #[error("AWS error {code} (HTTP {status}): {message}")]
    Other {
        code: String,
        message: String,
        status: u16,
    },
}

pub type AwsResult<T> = Result<T, AwsError>;

impl AwsError {
    /// A stable, short, machine-readable name for the variant, for the interface to switch on.
    pub fn kind(&self) -> &'static str {
        match self {
            AwsError::AccessDenied(_) => "access_denied",
            AwsError::NotFound(_) => "not_found",
            AwsError::VersionConflict(_) => "version_conflict",
            AwsError::SessionExpired(_) => "session_expired",
            AwsError::Throttled(_) => "throttled",
            AwsError::Network(_) => "network",
            AwsError::Invalid(_) => "invalid",
            AwsError::Other { .. } => "other",
        }
    }
}

impl From<reqwest::Error> for AwsError {
    fn from(error: reqwest::Error) -> Self {
        // `without_url` because a URL can carry a bucket, a key or an account id, and none of
        // that belongs in an error message that might end up on a screen or in a log.
        AwsError::Network(error.without_url().to_string())
    }
}

/// Reduce an AWS error type to its bare code.
///
/// JSON services spell it several ways: `ParameterNotFound`,
/// `com.amazonaws.ssm#ParameterNotFound`, or `ParameterNotFound:http://internal.amazon.com/...`
/// in the `x-amzn-ErrorType` header. All three mean the same thing.
pub(crate) fn bare_code(raw: &str) -> &str {
    let after_hash = raw.rsplit('#').next().unwrap_or(raw);
    after_hash.split(':').next().unwrap_or(after_hash).trim()
}

/// Fold an AWS error code (and the HTTP status it came with) into a variant.
///
/// The code wins when there is one, because it is precise; the status is the fallback for
/// responses that have no body at all, which is every error to a `HEAD` request.
pub(crate) fn classify(status: u16, code: &str, message: &str) -> AwsError {
    let code = bare_code(code);
    let message = if message.is_empty() {
        if code.is_empty() {
            format!("HTTP {status}")
        } else {
            code.to_string()
        }
    } else {
        message.to_string()
    };

    match code {
        "AccessDeniedException"
        | "AccessDenied"
        | "AllAccessDisabled"
        | "ForbiddenException"
        | "UnauthorizedOperation"
        | "KMSAccessDeniedException"
        | "InvalidAccessKeyId"
        | "AccountProblem" => AwsError::AccessDenied(message),

        "ParameterNotFound"
        | "ParameterVersionNotFound"
        | "InvalidResourceId"
        | "NoSuchBucket"
        | "NoSuchKey"
        | "NoSuchVersion"
        | "NotFound"
        | "ResourceNotFoundException" => AwsError::NotFound(message),

        "ParameterAlreadyExists" | "PreconditionFailed" | "ConditionalRequestConflict" => {
            AwsError::VersionConflict(message)
        }

        "ExpiredTokenException"
        | "ExpiredToken"
        | "UnrecognizedClientException"
        | "InvalidClientTokenId"
        | "InvalidToken"
        | "TokenRefreshRequired"
        | "UnauthorizedException"
        | "InvalidGrantException"
        | "invalid_grant"
        | "RequestExpired" => AwsError::SessionExpired(message),

        "ThrottlingException"
        | "Throttling"
        | "TooManyRequestsException"
        | "TooManyUpdates"
        | "SlowDown"
        | "RequestLimitExceeded"
        | "ThrottledException"
        | "ServiceUnavailable"
        | "ServiceUnavailableException" => AwsError::Throttled(message),

        "ValidationException"
        | "InvalidParameterValue"
        | "InvalidParameterException"
        | "InvalidRequest"
        | "InvalidRequestException"
        | "InvalidArgument"
        | "InvalidBucketName"
        | "KeyTooLongError"
        | "InvalidKeyId"
        | "InvalidAllowedPatternException"
        | "ParameterLimitExceeded"
        | "ParameterMaxVersionLimitExceeded"
        | "ParameterPatternMismatchException"
        | "HierarchyLevelLimitExceededException"
        | "HierarchyTypeMismatchException"
        | "UnsupportedParameterType"
        | "InvalidNextToken"
        | "InvalidFilterKey"
        | "InvalidFilterOption"
        | "InvalidFilterValue"
        | "TooManyTagsError"
        | "InvalidTypeNameException"
        | "PermanentRedirect"
        | "AuthorizationHeaderMalformed"
        | "IllegalLocationConstraintException"
        | "InvalidClientException"
        | "InvalidScopeException"
        | "UnsupportedGrantTypeException"
        | "UnauthorizedClientException"
        | "EntityTooLarge"
        // A signature that does not match is a wrong secret or a clock far off, and logging in
        // again fixes neither.
        | "SignatureDoesNotMatch"
        | "InvalidSignatureException"
        | "RequestTimeTooSkewed" => AwsError::Invalid(message),

        _ => match status {
            401 => AwsError::SessionExpired(message),
            403 => AwsError::AccessDenied(message),
            404 => AwsError::NotFound(message),
            409 | 412 => AwsError::VersionConflict(message),
            429 | 503 => AwsError::Throttled(message),
            _ => AwsError::Other {
                code: if code.is_empty() {
                    format!("HTTP{status}")
                } else {
                    code.to_string()
                },
                message,
                status,
            },
        },
    }
}

/// Read an error out of a JSON-protocol response (SSM, SSO-OIDC, the SSO portal).
///
/// The code lives in `__type`, or in the `x-amzn-ErrorType` header, or — for SSO-OIDC's OAuth
/// errors — in `error`. The message is `message`, `Message` or `error_description`.
pub(crate) fn from_json_response(status: u16, header_type: Option<&str>, body: &[u8]) -> AwsError {
    let parsed: serde_json::Value = serde_json::from_slice(body).unwrap_or(serde_json::Value::Null);
    let field = |name: &str| parsed.get(name).and_then(|v| v.as_str()).unwrap_or("");

    let code = [field("__type"), header_type.unwrap_or(""), field("error")]
        .into_iter()
        .find(|c| !c.is_empty())
        .unwrap_or("");
    let message = [
        field("message"),
        field("Message"),
        field("error_description"),
    ]
    .into_iter()
    .find(|m| !m.is_empty())
    .unwrap_or("");

    classify(status, code, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_code_reduces_to_the_same_name() {
        assert_eq!(bare_code("ParameterNotFound"), "ParameterNotFound");
        assert_eq!(
            bare_code("com.amazonaws.ssm#ParameterNotFound"),
            "ParameterNotFound"
        );
        assert_eq!(
            bare_code(
                "UnauthorizedException:http://internal.amazon.com/coral/com.amazon.switchboard/"
            ),
            "UnauthorizedException"
        );
    }

    #[test]
    fn a_json_error_body_becomes_the_matching_variant() {
        let body = br#"{"__type":"com.amazonaws.ssm#AccessDeniedException","message":"no"}"#;
        assert_eq!(
            from_json_response(400, None, body),
            AwsError::AccessDenied("no".into())
        );

        let body = br#"{"__type":"ExpiredTokenException","message":"The security token included in the request is expired"}"#;
        assert!(matches!(
            from_json_response(400, None, body),
            AwsError::SessionExpired(_)
        ));
    }

    #[test]
    fn the_header_carries_the_code_when_the_body_does_not() {
        let body = br#"{"message":"Session token not found or invalid"}"#;
        let error = from_json_response(401, Some("UnauthorizedException:http://x/"), body);
        assert_eq!(
            error,
            AwsError::SessionExpired("Session token not found or invalid".into())
        );
    }

    #[test]
    fn a_bare_status_still_means_something() {
        assert!(matches!(classify(404, "", ""), AwsError::NotFound(_)));
        assert!(matches!(classify(403, "", ""), AwsError::AccessDenied(_)));
        assert!(matches!(
            classify(418, "", ""),
            AwsError::Other { status: 418, .. }
        ));
    }
}
