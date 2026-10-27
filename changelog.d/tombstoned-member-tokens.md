### Security
- An API token or agent key whose member is removed (tombstoned, by import or a SCIM delete) no longer resolves, and neither does a token whose member belongs to another workspace. The check is part of the bearer lookup on both backends, so every surface that resolves a bearer refuses it (#1655).
