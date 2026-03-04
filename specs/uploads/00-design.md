# File Uploads & Attachments

File uploads let users attach files to task messages — screenshots, documents, or other reference material that gives the agent additional context.

## What Users Can Do

- **Upload a file** — select a file from disk and attach it to a message.
- **Paste from clipboard** — paste an image directly into the message input.
- **View attachments** — uploaded files appear inline in the conversation as content blocks.

## How It Works

1. The frontend uploads the file to S3 via a presigned URL obtained from the backend.
2. The upload metadata (S3 key, filename, content type, size) is stored in the database and linked to the message.
3. When the conversation is displayed, attachments are rendered inline — images are shown as thumbnails, other files as download links.
4. The agent receives attachment metadata as part of the message context and can reference uploaded content during its turn.

## Key Concepts

| Concept | Meaning |
|---------|---------|
| Upload | A file stored in S3 and linked to a task message |
| Presigned URL | A temporary S3 URL that allows the frontend to upload directly without proxying through the backend |
| Attachment | An uploaded file rendered inline in the conversation |
