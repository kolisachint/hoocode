//! Unit tests for the service-account assertion in `adc.rs`. The key pair is a
//! test-only fixture generated with `openssl genrsa 2048`; it protects nothing.

use super::*;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;

const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC3/IJ+FJ3KqzK0
qIBFjatGkNNXUtc3FaziS4rURy9KAEPbB7ItVT2edv6OJkJ7wqghhgzzdL1kTdnS
mSr/fcz3z11X8xVikjsBaT1wlJUK3K2Md+0pAA/s18TNCegNzGJAFVlqf0+veX6t
xqfL9/ChDi8Mqxibaa0trYQz5F1Rt8VWCdIFMvJov+Ugc1xe1xJ4iDHtX3aAtAtO
dHV8GqjViVIxl999b2OFr9xwXHwkWPyBh3/ZXrAknOjji6AUVRXuQ9dhonV2lKeT
qliNQSxiGRks3es1i4oOs8xikl54vVXnTt7UtvvLzZTAttbk5b+eEzKvJSSJ0Mlw
lRnNMfWNAgMBAAECggEAHEEIiBSh67TxLzmeIzA/kHFepy8XUkP/nevUaMd6vV4/
jWQH6nrH4O0sp8c71/YbbDSCCiQQDpZ/AB6Z8gzPXHHph39StiW3Ln8LGp4Sqk2b
0DG9h5mVGH3FIOinzHCp2OX+aoKdCdk6STTz4TiU6SK8unEO8RTgLZ7TKcXkx87S
O+YocdycsX4HiYtdiaMRxCuljyhHeSSPYPiEeW55vKaqSPR6FbWNqbDKt9lodO82
BLZjYUi3+29fiZfQtl2fTatQtqcdjkYEtnXpwdyNEA7Z7N2HCc31DixWNLG7+OXK
Q3jZFGWDBW7nSF6PTKZ+A3JMcGs/Qqv5KQQU01PLIQKBgQDre2Ko4wkwwaCM5v4X
5Zyf94sm4iXmx8Iz+yL3EcaqJJ7BPRpAPNt5N7GgHcYgSsmNVenk+p9SHQNdXllL
lI1ty/PtHQrvyu+GafcAAzq+6vqtleA4DbASM2JfbHqWV7NsLw3CuvS2FhpxMXXk
Ot0BsTbhmOf2b2VBtzfkjZknrQKBgQDIBHkzZYfJpmRPC9vwqpXTAq3T6+J+vTk6
cdNmV0cC97IubGX9kPqx6XTKnHbzFkRKGtlVg06RhAhl9SwQnfbwZfUsUjevP9fM
rqXgxTvNQ1UIc53GTQ1SMc4eZ/Ews6lBmlAKKwCmtqlyzN8W0SKtOkm3SrW7bkwr
0M96vlNBYQKBgQCZ1etoJZnVbkkyg50vg2NE5fTUHB1inNF0H/aBre05Ld9AomhJ
2vgOQqIZ1hme8vylBqFcY5Y9kJKoJoDzbULPndPm7Xg9mzfLeeJQ8cso+l11lVn7
hIWn/kWfzUBwgy7F2B+HPZesdNglZoh75zCGcH1H+Bph1fgybD1pZE/PqQKBgAOg
MSJBYOjC32afbGk8bmLY1ZXV3NE3VOY+Ufq08lCXdN9qj569Fg5LkRZrJ6zYZnyB
aBcovi/dGNngS+AxPNy2vG8FPoUqvxqyv8cHTEI4Sj5Q+jcyBl/LD1V1xOWBHDxY
M8M7bFOZVBEErbaYK+y/9b4g2JKBOVhB/MDY2mGBAoGAO4JQcIPy+a8G0SS+0dq1
extHAL0c7xj/JRdrIWv1iUeNZScOryJVEXuQI8g5PUX/rxTtw+7SQNhL7mKpstWg
Pq4nR/LH3WVYXJtJAG1IIbAuR5bgapoEHQMAki9oeynRhAzG5A4Ki4SOwntA6UXO
41NGy/HN4QIcJXVp8zWIdoI=
-----END PRIVATE KEY-----
"#;

const TEST_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAt/yCfhSdyqsytKiARY2r
RpDTV1LXNxWs4kuK1EcvSgBD2weyLVU9nnb+jiZCe8KoIYYM83S9ZE3Z0pkq/33M
989dV/MVYpI7AWk9cJSVCtytjHftKQAP7NfEzQnoDcxiQBVZan9Pr3l+rcany/fw
oQ4vDKsYm2mtLa2EM+RdUbfFVgnSBTLyaL/lIHNcXtcSeIgx7V92gLQLTnR1fBqo
1YlSMZfffW9jha/ccFx8JFj8gYd/2V6wJJzo44ugFFUV7kPXYaJ1dpSnk6pYjUEs
YhkZLN3rNYuKDrPMYpJeeL1V507e1Lb7y82UwLbW5OW/nhMyryUkidDJcJUZzTH1
jQIDAQAB
-----END PUBLIC KEY-----
"#;

const CLIENT_EMAIL: &str = "test-sa@test-project.iam.gserviceaccount.com";
const NOW: i64 = 1_700_000_000;

#[test]
fn service_account_jwt_is_rs256_and_verifies_with_public_key() {
    let jwt = service_account_jwt(TEST_PRIVATE_KEY, CLIENT_EMAIL, DEFAULT_TOKEN_URI, NOW)
        .expect("RS256 signing with a PKCS#8 RSA key must succeed");

    let header = decode_header(&jwt).expect("header decodes");
    assert_eq!(header.alg, Algorithm::RS256);
    assert_eq!(header.typ.as_deref(), Some("JWT"));

    // Check the signature against the public half, not just the header.
    let key = DecodingKey::from_rsa_pem(TEST_PUBLIC_KEY.as_bytes()).expect("public key parses");
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[DEFAULT_TOKEN_URI]);
    validation.set_required_spec_claims::<&str>(&[]);
    validation.validate_exp = false; // `NOW` is a fixed past timestamp
    let data = decode::<Value>(&jwt, &key, &validation).expect("signature verifies");

    assert_eq!(data.claims["iss"], CLIENT_EMAIL);
    assert_eq!(data.claims["scope"], CLOUD_PLATFORM_SCOPE);
    assert_eq!(data.claims["aud"], DEFAULT_TOKEN_URI);
    assert_eq!(data.claims["iat"], NOW);
    assert_eq!(data.claims["exp"], NOW + 3600);
}
