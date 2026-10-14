CREATE TABLE owners (
    id uuid PRIMARY KEY,
    email text NOT NULL,
    version bigint NOT NULL,
    CONSTRAINT owners_email_key UNIQUE (email)
);
