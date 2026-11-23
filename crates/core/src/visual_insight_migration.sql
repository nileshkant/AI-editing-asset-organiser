-- v20 writes structured evidence in the existing image asset JSON. In-flight
-- v1 requests may have been charged and must not be dispatched with v2.
UPDATE image_enrichment_usage SET status='unknown' WHERE status='reserved';
UPDATE image_enrichment_items
SET state='unknown_outcome',lease_owner=NULL,lease_until=NULL,error_code='OUTCOME_UNKNOWN'
WHERE state='dispatching';
UPDATE image_enrichment_items
SET state='revoked',lease_owner=NULL,lease_until=NULL,error_code='ANALYSIS_SCHEMA_CHANGED'
WHERE state IN ('pending','running');
UPDATE image_enrichment_jobs SET state='cancelled',error_code='ANALYSIS_SCHEMA_CHANGED'
WHERE state IN ('queued','running','paused');
