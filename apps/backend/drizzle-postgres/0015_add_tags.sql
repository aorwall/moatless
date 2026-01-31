-- Add tags table and task_tags join table for task categorization
CREATE TABLE "tags" (
  "id" serial PRIMARY KEY NOT NULL,
  "tag_id" text NOT NULL,
  "name" text NOT NULL,
  "description" text,
  "created_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX "tags_tag_id_idx" ON "tags" USING btree ("tag_id");
--> statement-breakpoint
CREATE UNIQUE INDEX "tags_name_idx" ON "tags" USING btree ("name");
--> statement-breakpoint
CREATE TABLE "task_tags" (
  "id" serial PRIMARY KEY NOT NULL,
  "task_id" text NOT NULL,
  "tag_id" text NOT NULL,
  "created_at" timestamp DEFAULT now() NOT NULL,
  CONSTRAINT "task_tags_task_id_tasks_task_id_fk" FOREIGN KEY ("task_id") REFERENCES "tasks"("task_id") ON DELETE CASCADE,
  CONSTRAINT "task_tags_tag_id_tags_tag_id_fk" FOREIGN KEY ("tag_id") REFERENCES "tags"("tag_id") ON DELETE CASCADE
);
--> statement-breakpoint
CREATE UNIQUE INDEX "task_tags_task_id_tag_id_idx" ON "task_tags" USING btree ("task_id", "tag_id");
--> statement-breakpoint
CREATE INDEX "task_tags_tag_id_idx" ON "task_tags" USING btree ("tag_id");
