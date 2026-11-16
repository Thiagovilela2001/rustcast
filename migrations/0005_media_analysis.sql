-- Loudness (EBU R128) + auto-cue, filled by the background analyzer. analyzed: 0 pending, 1 done, 2 failed.
ALTER TABLE media ADD COLUMN analyzed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media ADD COLUMN loudness_lufs REAL;
ALTER TABLE media ADD COLUMN peak_db REAL;
ALTER TABLE media ADD COLUMN gain_db REAL NOT NULL DEFAULT 0;
ALTER TABLE media ADD COLUMN cue_in_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media ADD COLUMN cue_out_ms INTEGER;
