### OAuth discovery says client metadata documents work

- **Fixed:** the authorization-server metadata now carries `client_id_metadata_document_supported: true`. Maidan already resolved a client named by the HTTPS URL of its metadata document (through the workspace's egress allowlist), but without the flag a client such as claude.ai assumed it had to register dynamically, which Maidan never offers, and refused to connect ("Automatic client registration isn't supported"). Found in Next 23's P6 run with a real claude.ai connector.
