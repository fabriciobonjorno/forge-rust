CREATE TABLE notes (
    id BLOB PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CONSTRAINT notes_title_length CHECK (length(title) BETWEEN 1 AND 200),
    version INTEGER NOT NULL DEFAULT 1 CHECK (version >= 1)
);
