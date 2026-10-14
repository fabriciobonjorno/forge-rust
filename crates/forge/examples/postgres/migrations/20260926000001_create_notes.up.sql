CREATE TABLE notes (
    id uuid PRIMARY KEY,
    title text NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1)
);
