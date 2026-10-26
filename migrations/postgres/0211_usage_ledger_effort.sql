-- C12: the reasoning effort a usage report's call asked for, reporter
-- evidence beside the model, so the ledger can price model and effort by
-- task class. Absent means the reporter did not say.
ALTER TABLE maidan_usage_ledger ADD COLUMN effort TEXT;
