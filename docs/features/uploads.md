# Uploads Architecture

## Overview

The uploads system allows users to upload files that can be referenced by AI agents in their tasks.

## Backend

The `registerUploadRoutes` function (`apps/backend/routes/uploads.ts`) sets up the file upload endpoint. When a file is uploaded, it is saved to the `data/uploads` directory with a unique ID. The file's metadata, including its original filename, path, and MIME type, is stored for later retrieval.

## Frontend

On the frontend, the `useUploadFile` hook (`apps/frontend/src/features/uploads/hooks/use-upload.ts`) provides a mutation for uploading files. The `ChatForm` component uses this hook to allow users to attach files to their messages.
