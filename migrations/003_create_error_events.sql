CREATE TABLE error_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    level TEXT NOT NULL DEFAULT 'error',
    title TEXT NOT NULL,
    message TEXT NOT NULL DEFAULT '',
    stack_trace JSONB,
    context JSONB NOT NULL DEFAULT '{}',
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_error_events_project_received ON error_events(project_id, received_at DESC);
CREATE INDEX idx_error_events_project_fingerprint ON error_events(project_id, fingerprint);
