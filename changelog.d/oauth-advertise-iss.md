### OAuth server metadata advertises iss

- **Fixed:** the authorization-server document now sets `authorization_response_iss_parameter_supported`, so clients know authorization responses carry `iss` (RFC 9207) and check it.
