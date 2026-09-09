-- One database per runtime, mirroring MVD's split. The control plane's database is
-- created by POSTGRES_DB; the rest are created here.
CREATE USER ih WITH PASSWORD 'ih';
CREATE DATABASE identityhub OWNER ih;

CREATE USER kc WITH PASSWORD 'kc';
CREATE DATABASE keycloak OWNER kc;
