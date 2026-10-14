CREATE TABLE notes (
    id BINARY(16) PRIMARY KEY,
    title VARCHAR(200) NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    CONSTRAINT notes_title_length CHECK (char_length(title) BETWEEN 1 AND 200),
    CONSTRAINT notes_version_positive CHECK (version >= 1)
);
