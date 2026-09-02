-- Retire the deleted Mobile push product without rewriting migration history.
DROP TRIGGER IF EXISTS events_enqueue_push_match ON events;
DROP FUNCTION IF EXISTS enqueue_push_match_job();

DROP TABLE IF EXISTS push_match_queue;
DROP TABLE IF EXISTS push_wake_outbox;
DROP TABLE IF EXISTS push_leases;

DROP TABLE IF EXISTS push_gateway_delegations;
DROP TABLE IF EXISTS push_gateway_installations;
DROP TABLE IF EXISTS push_gateway_challenges;
DROP TABLE IF EXISTS push_gateway_endpoint_quotas;
DROP TABLE IF EXISTS push_gateway_delivery_auth_replays;
DROP TABLE IF EXISTS push_gateway_delivery_request_replays;

DELETE FROM _operator_global_tables
WHERE table_name IN (
    'push_gateway_challenges',
    'push_gateway_installations',
    'push_gateway_delegations',
    'push_gateway_endpoint_quotas',
    'push_gateway_delivery_auth_replays',
    'push_gateway_delivery_request_replays'
);

-- Purge source events stored by the old lease handler, plus any rows inserted
-- through nonstandard paths, before the author-only read gate is removed.
-- Migration execution holds the schema/destruction advisory lock. Temporarily
-- remove the per-row fence so retired data in quiescing/fenced communities can
-- be erased, then restore the canonical trigger in the same transaction.
DROP TRIGGER IF EXISTS community_write_fence_events ON events;
DELETE FROM events WHERE kind = 30350;
SELECT attach_community_write_fence('events');

-- ponytail: leave the now-inert kind:30350 FTS exclusion in place to avoid a
-- whole-events-table generated-column rewrite; Issue #12's reset baseline
-- removes that final migration-era expression.
