### Fixed

- **Security:** the Helm chart refuses secret-shaped keys in ConfigMaps, CHANGE_ME or undersized content keys in Secrets, invalid HPA replica/CPU bounds, and empty or malformed PVC sizes before render.
