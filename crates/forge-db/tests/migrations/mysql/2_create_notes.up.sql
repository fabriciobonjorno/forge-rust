CREATE TABLE notes (
    id BINARY(16) PRIMARY KEY,
    owner_id BINARY(16) NOT NULL,
    title VARCHAR(100) NOT NULL,
    CONSTRAINT title_not_blank CHECK (CHAR_LENGTH(title) > 0),
    CONSTRAINT notes_owner_fk FOREIGN KEY (owner_id) REFERENCES owners (id)
);
