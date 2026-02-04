CREATE TABLE IF NOT EXISTS "task_message_content_blocks" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_message_id" integer NOT NULL,
	"sort_order" integer DEFAULT 0 NOT NULL,
	"block_type" text NOT NULL,
	"show_in_ui" boolean DEFAULT true NOT NULL,
	"text_content" text,
	"tool_use_id" text,
	"tool_result_content" text,
	"tool_result_is_error" boolean,
	"image_media_type" text,
	"image_data" text,
	"created_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
DO $$ BEGIN
 ALTER TABLE "task_message_content_blocks" ADD CONSTRAINT "task_message_content_blocks_task_message_id_task_messages_id_fk" FOREIGN KEY ("task_message_id") REFERENCES "public"."task_messages"("id") ON DELETE cascade ON UPDATE no action;
EXCEPTION
 WHEN duplicate_object THEN null;
END $$;
--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "task_message_content_blocks_message_idx" ON "task_message_content_blocks" USING btree ("task_message_id");
