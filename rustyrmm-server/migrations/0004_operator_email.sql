ALTER TABLE operators ADD COLUMN email text;
CREATE UNIQUE INDEX operators_email_lower ON operators (lower(email));
