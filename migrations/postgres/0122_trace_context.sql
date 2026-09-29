-- W3C trace context for a write and for the fan-out that follows it.
--
-- `traceparent` is the server span the request ran in (a child of the
-- caller's traceparent, or a root the server started). It is transport
-- metadata: it is not part of the event content hash, and a missing value
-- is work no request carried a trace into. Webhook, projector and
-- automation deliveries copy it at enqueue so a later send still names
-- the same parent after the request span has ended.
ALTER TABLE maidan_events ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_events ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_webhook_deliveries ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_webhook_deliveries ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_egress_outbox ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_egress_outbox ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_automation_deliveries ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_automation_deliveries ADD COLUMN tracestate TEXT;
