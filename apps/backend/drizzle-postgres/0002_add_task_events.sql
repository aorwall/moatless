CREATE TABLE "task_events" (
	"id" serial PRIMARY KEY NOT NULL,
	"event_id" text NOT NULL,
	"task_id" text NOT NULL,
	"type" text NOT NULL,
	"sub_type" text,
	"payload" text NOT NULL,
	"is_error" boolean DEFAULT false NOT NULL,
	"created_at" timestamp DEFAULT now() NOT NULL,
	"delivered_at" timestamp
);
--> statement-breakpoint
CREATE UNIQUE INDEX "task_events_event_id_idx" ON "task_events" USING btree ("event_id");
--> statement-breakpoint
CREATE INDEX "task_events_task_idx" ON "task_events" USING btree ("task_id");
--> statement-breakpoint
CREATE INDEX "task_events_pending_idx" ON "task_events" USING btree ("delivered_at");
