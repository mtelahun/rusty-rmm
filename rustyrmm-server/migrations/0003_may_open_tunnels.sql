-- Whether the Operator may open Tunnels to Endpoints. Existing Operators get false.
ALTER TABLE operators ADD COLUMN may_open_tunnels boolean NOT NULL DEFAULT false;
