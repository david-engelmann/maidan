-- W3C trace context. See the Postgres twin. Not part of the event hash.
ALTER TABLE maidan_events ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_events ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_webhook_deliveries ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_webhook_deliveries ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_egress_outbox ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_egress_outbox ADD COLUMN tracestate TEXT;

ALTER TABLE maidan_automation_deliveries ADD COLUMN traceparent TEXT;
ALTER TABLE maidan_automation_deliveries ADD COLUMN tracestate TEXT;
