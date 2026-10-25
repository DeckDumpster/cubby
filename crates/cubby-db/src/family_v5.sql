-- A video equivalent is a span of watch time, set per child by the parent (default 15 minutes).
-- A grant stamps the span it is charged at; grants begun before this stay NULL and keep the
-- fraction-of-the-video rule they were granted under.
ALTER TABLE budgets ADD COLUMN ve_minutes REAL;
ALTER TABLE play_grants ADD COLUMN ve_s REAL;
