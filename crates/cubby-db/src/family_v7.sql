-- Usage counts per local calendar day in the child's tz; the rolling window is retired.
ALTER TABLE budgets DROP COLUMN window_s;
