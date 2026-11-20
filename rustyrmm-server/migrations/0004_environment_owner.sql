-- Optional free-text label for the customer or site; NULL when unset.
ALTER TABLE environments ADD COLUMN owner text;
