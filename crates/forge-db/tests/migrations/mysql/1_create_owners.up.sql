CREATE TABLE owners (
    id BINARY(16) PRIMARY KEY,
    email VARCHAR(190) NOT NULL,
    version BIGINT NOT NULL,
    CONSTRAINT owners_email_key UNIQUE (email)
);
