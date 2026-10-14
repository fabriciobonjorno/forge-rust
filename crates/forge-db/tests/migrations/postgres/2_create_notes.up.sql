CREATE TABLE notes (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL,
    title text NOT NULL,
    CONSTRAINT title_not_blank CHECK (length(title) > 0),
    CONSTRAINT notes_owner_fk FOREIGN KEY (owner_id) REFERENCES owners (id)
);
