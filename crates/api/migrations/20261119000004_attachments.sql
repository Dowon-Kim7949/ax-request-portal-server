-- Uploaded files and the key that signs their download links.
--
-- The bytes live in AX_UPLOAD_DIR under the attachment's id; this table keeps
-- what the contract's `Attachment` reports and who uploaded it. Only the
-- uploader may put an attachment on a draft or a request. A request copies the
-- attachment's description into its details when it is filed, so nothing here
-- is deleted while a request may still show it.

create table attachments (
    id         text primary key,
    owner_id   uuid not null references users (id) on delete restrict,
    name       text not null,
    size       bigint not null check (size >= 0),
    mime_type  text not null,
    created_at timestamptz not null default now()
);

create index attachments_owner_id_idx on attachments (owner_id);

-- Keys the server makes for itself on first start (`file_urls`: the HMAC key
-- of download links), so a deployment has no extra secret to manage and links
-- keep working across restarts.
create table server_secrets (
    name       text primary key,
    value      bytea not null,
    created_at timestamptz not null default now()
);
