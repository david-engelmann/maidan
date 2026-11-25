-- A share ticket outlives the request that minted it by up to 48 hours, and
-- the person holding its link is outside the workspace. Each time the ticket
-- is used the server asks whether its creator could still mint it, so the
-- ticket has to name the credential it was minted with: revoking that token
-- then ends the ticket, as revoking a webhook creator's token ends the
-- webhook (0236).
--
-- Tickets minted before this column, and tickets minted on a browser session,
-- have no token to check; they are held to their creator's membership and
-- standing alone. Deleting a token row leaves its tickets in the same state.
ALTER TABLE maidan_share_tickets
    ADD COLUMN created_by_token UUID REFERENCES maidan_api_tokens(id) ON DELETE SET NULL;
