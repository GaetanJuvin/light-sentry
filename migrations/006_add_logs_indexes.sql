-- Speed up logs queries filtered by level
CREATE INDEX idx_logs_project_level_received ON logs(project_id, level, received_at DESC);
