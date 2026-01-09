-- Add role column to users table
-- Default to 'user' for existing users, 'admin' must be explicitly set
ALTER TABLE `users` ADD COLUMN `role` TEXT NOT NULL DEFAULT 'user';

-- Add role column to sessions table (cached from users)
ALTER TABLE `sessions` ADD COLUMN `role` TEXT NOT NULL DEFAULT 'user';
