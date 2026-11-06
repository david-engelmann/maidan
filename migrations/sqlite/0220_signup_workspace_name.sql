-- The display name chosen on the sign-up form, applied when the callback
-- creates the workspace. Null means the callback names it after the handle.
-- The handle is not known until the provider returns, so the choice has to
-- ride on this short-lived row.
ALTER TABLE maidan_oidc_pending ADD COLUMN workspace_name TEXT;
