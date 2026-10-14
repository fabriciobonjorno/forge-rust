CREATE TABLE owners (
    id BLOB PRIMARY KEY NOT NULL,
    email TEXT NOT NULL,
    version INTEGER NOT NULL,
    CONSTRAINT owners_email_key UNIQUE (email)
);
