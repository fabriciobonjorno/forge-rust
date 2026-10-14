CREATE TABLE notes (
    id BLOB PRIMARY KEY NOT NULL,
    owner_id BLOB NOT NULL,
    title TEXT NOT NULL,
    CONSTRAINT title_not_blank CHECK (length(title) > 0),
    CONSTRAINT notes_owner_fk FOREIGN KEY (owner_id) REFERENCES owners (id)
);
