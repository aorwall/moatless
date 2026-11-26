-- The schema migrations 0001-0253 built, squashed into one baseline.
-- `crates/core/src/db/migrations.rs::adopt_baseline` records it in place of
-- those rows on a database that applied them. Generated with `pg_dump`.

CREATE FUNCTION conversation_messages_agent_event() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.author = 'agent' AND NEW.task_event_id IS NOT NULL AND NOT NEW.frozen
       AND NOT EXISTS (
        SELECT 1 FROM task_events
         WHERE event_id = NEW.task_event_id AND type = 'task' AND sub_type = 'binding_reply_sent'
    ) THEN
        RAISE EXCEPTION 'agent Message % names task event % that is not a binding reply',
            NEW.uuid, NEW.task_event_id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION trajectory_sessions_fork_same_harness() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
    IF NEW.forked_from_event_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
          FROM trajectory_events e
          JOIN trajectory_sessions s ON s.id = e.session_id
         WHERE e.id = NEW.forked_from_event_id AND s.harness = NEW.harness
    ) THEN
        RAISE EXCEPTION 'trajectory session % forks an event of another harness', NEW.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END;
$$;

CREATE TABLE adapter_app_audit_events (
    id text NOT NULL,
    adapter_app_id text,
    adapter_kind text NOT NULL,
    app_key text NOT NULL,
    actor_id text NOT NULL,
    action text NOT NULL,
    result text NOT NULL,
    detail text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE adapter_app_migrations (
    name text NOT NULL,
    completed_at timestamp with time zone DEFAULT now() NOT NULL,
    detail text
);

CREATE TABLE adapter_app_secrets (
    adapter_app_id text NOT NULL,
    name text NOT NULL,
    value_ciphertext bytea NOT NULL,
    value_nonce bytea NOT NULL,
    encrypted_data_key bytea NOT NULL,
    kms_key_id text NOT NULL,
    encryption_version integer DEFAULT 1 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT adapter_app_secrets_name_format_chk CHECK ((name ~ '^[a-z][a-z0-9_]*$'::text))
);

CREATE TABLE adapter_apps (
    id text NOT NULL,
    adapter_kind text NOT NULL,
    app_key text NOT NULL,
    display_name text NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    config_json jsonb DEFAULT '{}'::jsonb NOT NULL,
    secret_fingerprints jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by text NOT NULL,
    updated_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    revision integer DEFAULT 1 NOT NULL,
    CONSTRAINT adapter_apps_key_format_chk CHECK ((app_key ~ '^[A-Za-z0-9][A-Za-z0-9_-]*$'::text)),
    CONSTRAINT adapter_apps_kind_format_chk CHECK ((adapter_kind ~ '^[a-z][a-z0-9_]*$'::text)),
    CONSTRAINT adapter_apps_revision_chk CHECK ((revision > 0))
);

CREATE TABLE adapter_connection_events (
    connection_id text NOT NULL,
    event_type text NOT NULL,
    subject_key text NOT NULL,
    external_id text NOT NULL,
    received_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE adapter_connections (
    id text NOT NULL,
    adapter_kind text NOT NULL,
    connection_kind text NOT NULL,
    external_account_id text NOT NULL,
    credential_ref text,
    webhook_secret text,
    transport_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_by text NOT NULL,
    scope text DEFAULT 'user'::text NOT NULL,
    owner_user_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT adapter_connections_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL))))
);

COMMENT ON TABLE adapter_connections IS 'Adapter connection records: credentials and sender/account metadata for one Adapter.';

CREATE TABLE adapter_delivery (
    id text NOT NULL,
    adapter text NOT NULL,
    kind text NOT NULL,
    event_id text NOT NULL,
    payload jsonb NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    max_attempts integer NOT NULL,
    next_attempt_at timestamp with time zone DEFAULT now() NOT NULL,
    claimed_at timestamp with time zone,
    claimed_by text,
    last_error text,
    admitted_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT adapter_delivery_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'claimed'::text, 'succeeded'::text, 'failed'::text])))
);

COMMENT ON COLUMN adapter_delivery.adapter IS 'The Adapter this delivery arrived on ("slack", "telegram", "github", "linear", "webhook").';

COMMENT ON COLUMN adapter_delivery.event_id IS 'The sender''s own delivery identifier, never one the backend generates — except for the generic Webhook Adapter, whose caller may supply one and gets a freshly minted one when it does not.';

CREATE TABLE adapter_event_receipts (
    adapter_kind text NOT NULL,
    external_event_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_key text NOT NULL,
    task_id text NOT NULL,
    first_seen_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE adapter_event_receipts IS 'Deduplication receipts for Adapter events delivered to Tasks.';

CREATE TABLE adapter_task_creation_claims (
    loop_id text NOT NULL,
    subject_kind text NOT NULL,
    subject_key text NOT NULL,
    adapter_kind text NOT NULL,
    external_event_id text,
    task_id text,
    claimed_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE adapter_task_creation_claims IS 'Single-winner claim for creating a task from an inbound adapter event. One row per (loop, subject); the winner records its task_id, losers retry and deliver to it.';

CREATE TABLE api_keys (
    id text NOT NULL,
    user_id text NOT NULL,
    name text NOT NULL,
    key_hash text NOT NULL,
    key_prefix text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone,
    expires_at timestamp with time zone,
    revoked boolean DEFAULT false,
    scopes text,
    group_id text,
    idle_ttl_secs bigint
);

CREATE TABLE auth_audit_events (
    id uuid NOT NULL,
    occurred_at timestamp with time zone DEFAULT now() NOT NULL,
    user_id uuid,
    auth_session_id uuid,
    event text NOT NULL,
    authentication_method text,
    provider_id text,
    success boolean NOT NULL,
    ip_prefix inet,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL
);

CREATE TABLE auth_rate_limits (
    bucket text NOT NULL,
    subject_hash bytea NOT NULL,
    window_started_at timestamp with time zone NOT NULL,
    attempts integer DEFAULT 0 NOT NULL
);

CREATE TABLE auth_sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    token_hash bytea NOT NULL,
    csrf_token_hash bytea NOT NULL,
    authenticated_via text NOT NULL,
    group_id text,
    created_at timestamp with time zone NOT NULL,
    last_seen_at timestamp with time zone NOT NULL,
    idle_expires_at timestamp with time zone NOT NULL,
    absolute_expires_at timestamp with time zone NOT NULL,
    revoked_at timestamp with time zone,
    ip_prefix inet,
    user_agent text,
    CONSTRAINT auth_sessions_lifetime_check CHECK ((idle_expires_at <= absolute_expires_at))
);

CREATE TABLE auth_transactions (
    id uuid NOT NULL,
    provider_id text NOT NULL,
    state_hash bytea NOT NULL,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    redirect_after_login text,
    cli_callback_port integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone
);

CREATE TABLE collection_record_changes (
    collection_id text NOT NULL,
    seq bigint NOT NULL,
    key text NOT NULL
);

CREATE TABLE collection_records (
    collection_id text NOT NULL,
    key text NOT NULL,
    data jsonb NOT NULL,
    submitted_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    writer text DEFAULT 'page'::text NOT NULL
);

CREATE TABLE conversation_activities (
    activity_id uuid NOT NULL,
    task_id text NOT NULL,
    "position" bigint NOT NULL,
    ordinal integer DEFAULT 0 NOT NULL,
    message_uuid uuid,
    parent_tool_use_id text,
    kind text NOT NULL,
    status text NOT NULL,
    tool_use_id text,
    summary text NOT NULL,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    detail text,
    is_error boolean DEFAULT false NOT NULL,
    trajectory_event_id uuid,
    result_event_id uuid,
    inbox_id uuid,
    task_event_id text,
    frozen boolean DEFAULT false NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    result jsonb,
    typed_result jsonb,
    CONSTRAINT conversation_activities_check1 CHECK (((result_event_id IS NULL) OR (status <> 'in_progress'::text))),
    CONSTRAINT conversation_activities_kind_check CHECK ((kind = ANY (ARRAY['tool_call'::text, 'thinking'::text, 'user_input_requested'::text, 'plan_updated'::text, 'user_input_resolved'::text, 'sandbox_status'::text, 'sandbox_warning'::text, 'sandbox_error'::text, 'context_window_updated'::text, 'agent_switched'::text, 'compaction'::text, 'artifact_shown'::text, 'secret_requested'::text, 'secret_request_answered'::text]))),
    CONSTRAINT conversation_activities_source_check CHECK ((frozen OR
CASE
    WHEN (kind = 'tool_call'::text) THEN ((trajectory_event_id IS NOT NULL) AND (tool_use_id IS NOT NULL) AND (num_nonnulls(inbox_id, task_event_id) = 0))
    WHEN (kind = ANY (ARRAY['thinking'::text, 'user_input_requested'::text, 'plan_updated'::text, 'compaction'::text])) THEN ((trajectory_event_id IS NOT NULL) AND (num_nonnulls(tool_use_id, result_event_id, inbox_id, task_event_id) = 0))
    WHEN (kind = 'user_input_resolved'::text) THEN ((inbox_id IS NOT NULL) AND (num_nonnulls(trajectory_event_id, result_event_id, tool_use_id, task_event_id) = 0))
    ELSE ((task_event_id IS NOT NULL) AND (num_nonnulls(trajectory_event_id, result_event_id, tool_use_id, inbox_id) = 0))
END)),
    CONSTRAINT conversation_activities_status_check CHECK ((status = ANY (ARRAY['in_progress'::text, 'completed'::text, 'failed'::text])))
);

CREATE TABLE conversation_frozen (
    task_id text NOT NULL,
    frozen_at timestamp with time zone DEFAULT now() NOT NULL,
    open_turn integer,
    open_since timestamp with time zone
);

CREATE TABLE conversation_messages (
    uuid uuid NOT NULL,
    task_id text NOT NULL,
    "position" bigint NOT NULL,
    author text NOT NULL,
    kind text NOT NULL,
    text text,
    text_parts text[],
    parent_tool_use_id text,
    turn_number integer,
    harness text,
    model text,
    usage jsonb,
    inbox_id uuid,
    trajectory_event_id uuid,
    task_event_id text,
    frozen boolean DEFAULT false NOT NULL,
    legacy jsonb,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT conversation_messages_author_check CHECK ((author = ANY (ARRAY['user'::text, 'agent'::text, 'system'::text]))),
    CONSTRAINT conversation_messages_check CHECK ((frozen OR (num_nonnulls(inbox_id, trajectory_event_id, task_event_id) = 1))),
    CONSTRAINT conversation_messages_check1 CHECK ((frozen OR
CASE
    WHEN (author = 'user'::text) THEN ((kind = 'text'::text) AND (inbox_id IS NOT NULL))
    WHEN (author = 'agent'::text) THEN ((kind = 'text'::text) AND (inbox_id IS NULL) AND ((trajectory_event_id IS NOT NULL) = (harness IS NOT NULL)))
    ELSE ((kind = ANY (ARRAY['error'::text, 'turn_result'::text])) AND (task_event_id IS NOT NULL))
END)),
    CONSTRAINT conversation_messages_check2 CHECK ((frozen OR (usage IS NULL) OR (kind = 'turn_result'::text) OR (author = 'agent'::text))),
    CONSTRAINT conversation_messages_check3 CHECK ((frozen = (legacy IS NOT NULL))),
    CONSTRAINT conversation_messages_harness_check CHECK ((harness = ANY (ARRAY['claude-code'::text, 'codex'::text, 'pi'::text, 'mini-swe'::text, 'acp'::text]))),
    CONSTRAINT conversation_messages_kind_check CHECK ((kind = ANY (ARRAY['text'::text, 'error'::text, 'turn_result'::text])))
);

CREATE TABLE conversation_usage (
    trajectory_event_id uuid,
    message_uuid uuid,
    task_id text NOT NULL,
    parent_tool_use_id text,
    input_tokens integer DEFAULT 0 NOT NULL,
    output_tokens integer DEFAULT 0 NOT NULL,
    cache_creation_input_tokens integer DEFAULT 0 NOT NULL,
    cache_read_input_tokens integer DEFAULT 0 NOT NULL,
    CONSTRAINT conversation_usage_check CHECK ((num_nonnulls(trajectory_event_id, message_uuid) = 1)),
    CONSTRAINT conversation_usage_check1 CHECK (((trajectory_event_id IS NULL) OR (parent_tool_use_id IS NULL)))
);

CREATE TABLE device_pairing_codes (
    id text NOT NULL,
    code_hash text NOT NULL,
    user_id text NOT NULL,
    group_id text,
    label text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    consumed_at timestamp with time zone
);

CREATE TABLE feedback (
    id bigint NOT NULL,
    feedback_id text NOT NULL,
    task_id text NOT NULL,
    message_uuid text,
    tool_use_id text,
    source text NOT NULL,
    surface text NOT NULL,
    comment text NOT NULL,
    metadata jsonb,
    created_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    category text,
    status text DEFAULT 'open'::text NOT NULL,
    status_note text,
    status_updated_by text,
    status_updated_at timestamp with time zone
);

CREATE SEQUENCE feedback_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE feedback_id_seq OWNED BY feedback.id;

CREATE TABLE file_metadata (
    file_id text NOT NULL,
    filename text NOT NULL,
    mime_type text NOT NULL,
    size bigint NOT NULL,
    uploaded_at timestamp with time zone DEFAULT now() NOT NULL,
    scope text DEFAULT 'global'::text NOT NULL,
    owner_user_id text,
    created_by text DEFAULT 'system'::text,
    CONSTRAINT file_metadata_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL))))
);

CREATE TABLE forgejo_instances (
    id text NOT NULL,
    host text NOT NULL,
    client_id text NOT NULL,
    client_secret_ciphertext bytea NOT NULL,
    client_secret_nonce bytea NOT NULL,
    client_secret_data_key bytea NOT NULL,
    kms_key_id text NOT NULL,
    encryption_version integer DEFAULT 1 NOT NULL,
    created_by text NOT NULL,
    updated_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT forgejo_instances_host_format_chk CHECK ((host ~ '^[a-z0-9][a-z0-9.-]*(:[0-9]+)?$'::text))
);

CREATE TABLE harness_cooldowns (
    user_id text NOT NULL,
    harness text NOT NULL,
    until timestamp with time zone NOT NULL,
    reason text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE inbound_message_buffer (
    id text NOT NULL,
    seq bigint NOT NULL,
    binding_id text NOT NULL,
    loop_id text NOT NULL,
    adapter_label text NOT NULL,
    message jsonb NOT NULL,
    flush_at timestamp with time zone NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    claimed_at timestamp with time zone,
    claimed_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

COMMENT ON TABLE inbound_message_buffer IS 'Inbound adapter events held for their binding''s buffer window, flushed as one combined task message.';

CREATE SEQUENCE inbound_message_buffer_seq_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE inbound_message_buffer_seq_seq OWNED BY inbound_message_buffer.seq;

CREATE TABLE loop_configs (
    loop_id text NOT NULL,
    routing_mode text DEFAULT 'by_subject'::text NOT NULL,
    ongoing_task_id text,
    prompt text,
    task_name text,
    tag_ids text[] DEFAULT '{}'::text[] NOT NULL,
    run_as_user_id text,
    agent_profile text DEFAULT 'claude-code'::text NOT NULL,
    close_after_settled_days integer,
    skills text[] DEFAULT '{}'::text[] NOT NULL,
    model text,
    workspace_id text NOT NULL,
    node_selector jsonb,
    stop_when_done boolean,
    message_buffer_seconds integer,
    fallback_to_other_agent boolean DEFAULT true NOT NULL,
    memory_preload text[] DEFAULT '{}'::text[] NOT NULL,
    settle_after_hours integer,
    room jsonb,
    agent_properties jsonb DEFAULT '{}'::jsonb NOT NULL
);

COMMENT ON TABLE loop_configs IS 'Shared task-spawn configuration for a Loop.';

CREATE TABLE loops (
    id text NOT NULL,
    name text NOT NULL,
    kind text NOT NULL,
    active boolean DEFAULT true NOT NULL,
    scope text DEFAULT 'user'::text NOT NULL,
    owner_user_id text,
    created_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    source_repository_id text,
    source_key text,
    config_source text DEFAULT 'manual'::text NOT NULL,
    source_config_path text,
    deleted boolean DEFAULT false NOT NULL,
    CONSTRAINT loops_config_source_check CHECK ((config_source = ANY (ARRAY['git'::text, 'manual'::text]))),
    CONSTRAINT loops_name_not_blank_check CHECK ((btrim(name) <> ''::text)),
    CONSTRAINT loops_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL))))
);

COMMENT ON TABLE loops IS 'Standing definitions of recurring agent work.';

CREATE TABLE memory_entries (
    entry_id text NOT NULL,
    scope_kind text NOT NULL,
    scope_id text NOT NULL,
    key text NOT NULL,
    summary text NOT NULL,
    body text DEFAULT ''::text NOT NULL,
    data jsonb,
    tags text[] DEFAULT '{}'::text[] NOT NULL,
    citations text[] DEFAULT '{}'::text[] NOT NULL,
    version integer DEFAULT 1 NOT NULL,
    ttl_seconds bigint,
    expires_at timestamp with time zone,
    stale_after_seconds bigint,
    verified_at timestamp with time zone,
    claimed_by_task_id text,
    claim_expires_at timestamp with time zone,
    updated_by_task_id text,
    updated_by_loop_id text,
    updated_by_user_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    search tsvector GENERATED ALWAYS AS ((to_tsvector('simple'::regconfig, ((((key || ' '::text) || summary) || ' '::text) || body)) || array_to_tsvector(tags))) STORED,
    created_by_task_id text,
    created_by_loop_id text,
    created_by_user_id text,
    CONSTRAINT memory_entries_scope_kind_check CHECK ((scope_kind = ANY (ARRAY['task'::text, 'loop'::text, 'workspace'::text, 'repository'::text, 'user'::text, 'global'::text]))),
    -- `BETWEEN`, as 0200 wrote it: the `AND` pg_dump prints parses flatter.
    CONSTRAINT memory_entries_summary_check CHECK (char_length(summary) BETWEEN 1 AND 160 AND position(E'\n' IN summary) = 0)
);

CREATE TABLE memory_events (
    event_id bigint NOT NULL,
    entry_id text NOT NULL,
    op text NOT NULL,
    version integer NOT NULL,
    text text,
    task_id text,
    user_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT memory_events_op_check CHECK ((op = ANY (ARRAY['put'::text, 'append'::text, 'verify'::text, 'claim'::text, 'release'::text])))
);

CREATE SEQUENCE memory_events_event_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE memory_events_event_id_seq OWNED BY memory_events.event_id;

CREATE TABLE memory_reads (
    entry_id text NOT NULL,
    task_id text NOT NULL,
    version integer NOT NULL,
    read_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE memory_seen (
    task_id text NOT NULL,
    seen_at timestamp with time zone,
    staged_at timestamp with time zone
);

CREATE TABLE oidc_identities (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    provider_id text NOT NULL,
    issuer text NOT NULL,
    subject text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_login_at timestamp with time zone
);

CREATE TABLE plugin_activations (
    plugin_id text NOT NULL,
    skill_name text,
    user_id uuid,
    enabled boolean NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    workspace_id text,
    always boolean DEFAULT false NOT NULL,
    frontload boolean DEFAULT false NOT NULL,
    CONSTRAINT plugin_activations_one_subject CHECK (((user_id IS NULL) OR (workspace_id IS NULL)))
);

CREATE TABLE plugin_skills (
    plugin_id text NOT NULL,
    name text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    path text DEFAULT ''::text NOT NULL
);

CREATE TABLE plugins (
    id text NOT NULL,
    name text NOT NULL,
    git_url text NOT NULL,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    scope text DEFAULT 'global'::text NOT NULL,
    owner_user_id text,
    created_by text DEFAULT 'system'::text,
    synced_version text,
    synced_at timestamp with time zone,
    source text DEFAULT 'manual'::text NOT NULL,
    disabled_at timestamp with time zone,
    CONSTRAINT plugins_source_check CHECK ((source = ANY (ARRAY['manual'::text, 'deployment'::text])))
);

CREATE TABLE preview_tab_changes (
    "position" bigint NOT NULL,
    origin text NOT NULL,
    task_id text NOT NULL,
    tab_id text NOT NULL,
    url text NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE preview_tab_position (
    id boolean DEFAULT true NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL,
    CONSTRAINT preview_tab_position_is_one_row CHECK (id)
);

CREATE TABLE provider_tokens (
    id integer NOT NULL,
    user_id text,
    provider text NOT NULL,
    access_token text NOT NULL,
    refresh_token text,
    token_type text,
    metadata jsonb,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    auth_method text DEFAULT 'manual'::text,
    oauth_scope text,
    provider_user_id text,
    provider_login text,
    connected_at timestamp with time zone DEFAULT now(),
    expires_at timestamp with time zone,
    scope text NOT NULL,
    owner_user_id text,
    CONSTRAINT provider_tokens_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL)))),
    CONSTRAINT provider_tokens_scope_chk CHECK ((scope = ANY (ARRAY['global'::text, 'user'::text])))
);

CREATE SEQUENCE provider_tokens_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE provider_tokens_id_seq OWNED BY provider_tokens.id;

CREATE SEQUENCE pull_request_host_changes
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

CREATE TABLE record_collections (
    collection_id text NOT NULL,
    scope_kind text NOT NULL,
    scope_id text NOT NULL,
    name text NOT NULL,
    artifact_id text,
    record_seq bigint DEFAULT 0 NOT NULL,
    created_by_task_id text,
    created_by_user_id text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT record_collections_check CHECK (((scope_kind = 'artifact'::text) = (artifact_id IS NOT NULL))),
    CONSTRAINT record_collections_scope_kind_check CHECK ((scope_kind = ANY (ARRAY['artifact'::text, 'task'::text, 'loop'::text, 'workspace'::text, 'repository'::text, 'user'::text, 'global'::text])))
);

CREATE TABLE repositories (
    id text NOT NULL,
    kind text NOT NULL,
    name text NOT NULL,
    description text,
    remote_url text,
    provider text,
    default_branch text,
    deleted boolean,
    system_prompt text,
    setup_commands jsonb,
    docker_image text,
    env_vars jsonb,
    build_command text,
    build_output_directory text DEFAULT 'dist'::text,
    created_by text,
    created_at timestamp with time zone,
    updated_at timestamp with time zone,
    scope text DEFAULT 'user'::text NOT NULL,
    resources jsonb,
    owner_user_id text,
    docker_access boolean,
    sandbox_service_account text,
    network_profile text,
    CONSTRAINT repositories_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL)))),
    CONSTRAINT repositories_scope_chk CHECK ((scope = ANY (ARRAY['global'::text, 'user'::text])))
);

CREATE TABLE sandbox_unreported_commands (
    task_id text NOT NULL,
    commands jsonb DEFAULT '[]'::jsonb NOT NULL,
    revision bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE sandboxes (
    desired_state text DEFAULT 'stopped'::text NOT NULL,
    stop_reason text,
    desired_state_updated_at timestamp with time zone DEFAULT '1970-01-01 00:00:00+00'::timestamp with time zone NOT NULL,
    started_at timestamp with time zone,
    archive_retention text,
    operation_kind text,
    operation_generation bigint,
    operation_started_at timestamp with time zone,
    operation_seq bigint DEFAULT 0 NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    storage_reclaimed_at timestamp with time zone,
    sandbox_id text NOT NULL,
    workspace_id text NOT NULL,
    owner_user_id text,
    run_as_user_id text,
    branch text,
    headless boolean DEFAULT false NOT NULL,
    pending_work_at timestamp with time zone,
    agent_profile text,
    use_tmux boolean DEFAULT false NOT NULL,
    running_commands integer DEFAULT 0 NOT NULL,
    node_selector jsonb,
    stop_when_done boolean DEFAULT false NOT NULL,
    token_generation bigint DEFAULT 0 NOT NULL,
    driver_user_id text,
    service_account text,
    CONSTRAINT sandboxes_archive_retention_check CHECK (((archive_retention IS NULL) OR (archive_retention = ANY (ARRAY['retain_archives'::text, 'delete_archives'::text])))),
    CONSTRAINT sandboxes_desired_state_check CHECK ((desired_state = ANY (ARRAY['running'::text, 'stopped'::text, 'removed'::text]))),
    CONSTRAINT sandboxes_operation_kind_check CHECK (((operation_kind IS NULL) OR (operation_kind = ANY (ARRAY['start'::text, 'stop'::text, 'restart'::text, 'redeploy'::text, 'remove'::text])))),
    CONSTRAINT sandboxes_stop_reason_check CHECK (((stop_reason IS NULL) OR (stop_reason = ANY (ARRAY['user'::text, 'idle_reaper'::text, 'cleanup'::text, 'system'::text, 'error'::text]))))
);

CREATE TABLE schedules (
    loop_id text NOT NULL,
    cron_expression text NOT NULL,
    timezone text DEFAULT 'UTC'::text NOT NULL,
    message_template text NOT NULL,
    last_fired_at timestamp with time zone,
    next_fire_at timestamp with time zone NOT NULL,
    locked_by text,
    locked_until timestamp with time zone
);

COMMENT ON TABLE schedules IS 'Schedule firing definitions for Loops.';

CREATE TABLE secret_audit_events (
    id text NOT NULL,
    secret_id text,
    actor_id text NOT NULL,
    action text NOT NULL,
    scope_type text NOT NULL,
    scope_id text,
    key text NOT NULL,
    result text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE secrets (
    id text NOT NULL,
    key text NOT NULL,
    kind text NOT NULL,
    provider text,
    value_ciphertext bytea,
    encryption_nonce bytea,
    encrypted_data_key bytea,
    external_ref text,
    kms_key_id text,
    encryption_version integer DEFAULT 1 NOT NULL,
    created_by text NOT NULL,
    updated_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    last_used_at timestamp with time zone,
    version integer DEFAULT 1 NOT NULL,
    scope text NOT NULL,
    owner_user_id text,
    enabled boolean DEFAULT true NOT NULL,
    CONSTRAINT secrets_encryption_metadata_chk CHECK (((value_ciphertext IS NULL) OR ((encryption_nonce IS NOT NULL) AND (encrypted_data_key IS NOT NULL) AND (kms_key_id IS NOT NULL) AND (encryption_version > 0)))),
    CONSTRAINT secrets_key_format_chk CHECK ((key ~ '^[A-Z_][A-Z0-9_]*$'::text)),
    CONSTRAINT secrets_kind_chk CHECK ((kind = ANY (ARRAY['env'::text, 'provider_token'::text, 'runtime_file'::text, 'deployment'::text, 'internal'::text]))),
    CONSTRAINT secrets_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL)))),
    CONSTRAINT secrets_source_chk CHECK ((((value_ciphertext IS NOT NULL) AND (external_ref IS NULL)) OR ((value_ciphertext IS NULL) AND (external_ref IS NOT NULL)))),
    CONSTRAINT secrets_version_chk CHECK ((version > 0))
);

CREATE TABLE subscriptions (
    loop_id text NOT NULL,
    adapter_kind text NOT NULL,
    connection_id text NOT NULL,
    source_matcher text NOT NULL,
    source_name text,
    event_filter jsonb DEFAULT '[]'::jsonb NOT NULL,
    adapter_config jsonb,
    allow_direct_send boolean DEFAULT false NOT NULL,
    close_task_on_close boolean DEFAULT true NOT NULL,
    notify_on_success boolean DEFAULT false NOT NULL
);

COMMENT ON TABLE subscriptions IS 'Adapter-event firing definitions for Loops.';

CREATE TABLE tags (
    id integer NOT NULL,
    tag_id text NOT NULL,
    name text NOT NULL,
    description text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE tags_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE tags_id_seq OWNED BY tags.id;

CREATE TABLE task_artifact_versions (
    artifact_id text NOT NULL,
    version integer NOT NULL,
    published_at timestamp with time zone DEFAULT now() NOT NULL,
    turn_number integer
);

CREATE TABLE task_artifacts (
    artifact_id text NOT NULL,
    task_id text NOT NULL,
    slug text NOT NULL,
    title text NOT NULL,
    reserved_version integer DEFAULT 1 NOT NULL,
    latest_version integer DEFAULT 0 NOT NULL,
    published_versions integer[] DEFAULT '{}'::integer[] NOT NULL,
    created_by text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    inline_versions integer[] DEFAULT '{}'::integer[] NOT NULL,
    accepts_data boolean DEFAULT false NOT NULL,
    notify_on_data boolean DEFAULT false NOT NULL,
    visibility text DEFAULT 'public'::text NOT NULL,
    record_rules jsonb,
    collections jsonb DEFAULT '[]'::jsonb NOT NULL,
    forked_from_artifact_id text,
    forked_from_version integer
);

CREATE TABLE task_bindings (
    id text NOT NULL,
    task_id text NOT NULL,
    adapter_kind text NOT NULL,
    connection_id text NOT NULL,
    loop_id text,
    subject_kind text NOT NULL,
    subject_key text NOT NULL,
    destination_config jsonb DEFAULT '{}'::jsonb NOT NULL,
    metadata jsonb DEFAULT '{}'::jsonb NOT NULL,
    last_external_event_id text,
    origin text DEFAULT 'ingress'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    last_event_at timestamp with time zone DEFAULT now() NOT NULL,
    external_status text,
    external_state_kind text,
    external_status_raw jsonb,
    external_status_updated_at timestamp with time zone,
    active boolean DEFAULT true NOT NULL,
    deactivated_at timestamp with time zone,
    external_review_decision text,
    external_mergeable_state text,
    watching boolean DEFAULT true NOT NULL
);

CREATE TABLE task_conversation_changes (
    task_id text NOT NULL,
    "position" bigint NOT NULL,
    message_uuid text,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL,
    activity_id text,
    reverted_to_turn integer
);

CREATE TABLE task_conversation_position (
    task_id text NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL
);

CREATE TABLE task_event_changes (
    "position" bigint NOT NULL,
    event_id text NOT NULL,
    task_id text NOT NULL,
    event text NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE task_event_dispatch_queue (
    "position" bigint NOT NULL,
    claimed_at timestamp with time zone
);

CREATE TABLE task_event_position (
    id boolean DEFAULT true NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL,
    CONSTRAINT task_event_position_is_one_row CHECK (id)
);

CREATE TABLE task_events (
    id bigint NOT NULL,
    event_id text NOT NULL,
    task_id text NOT NULL,
    type text NOT NULL,
    sub_type text NOT NULL,
    is_error boolean DEFAULT false NOT NULL,
    agent_profile text,
    turn_number integer,
    payload jsonb DEFAULT '{}'::jsonb NOT NULL,
    "timestamp" text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE task_events_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE task_events_id_seq OWNED BY task_events.id;

CREATE TABLE task_follows (
    user_id text NOT NULL,
    task_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    settled_override text,
    settled_at timestamp with time zone,
    snoozed_until timestamp with time zone,
    snoozed_at timestamp with time zone,
    pinned_at timestamp with time zone,
    pin_order_key text,
    active_order_key text,
    auto_settle_disabled_at timestamp with time zone,
    snooze_woke_at timestamp with time zone
);

CREATE TABLE task_forks (
    task_id text NOT NULL,
    source_task_id text NOT NULL,
    at_turn integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    source_event_id uuid NOT NULL
);

COMMENT ON COLUMN task_forks.source_task_id IS 'The Task the session is branched from: the parent for a fork, the Task itself for a rewind. No foreign key, so it survives the parent being deleted.';

COMMENT ON COLUMN task_forks.at_turn IS 'The source turn the cut keeps. NULL only for a fork at head; a rewind always names one.';

CREATE TABLE task_inbox (
    id uuid NOT NULL,
    task_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    text text NOT NULL,
    file_ids text[] DEFAULT '{}'::text[] NOT NULL,
    command_name text,
    delivery_options jsonb,
    answers_tool_use_id text,
    origin text NOT NULL,
    sent_by text,
    source_task_id text,
    external_sender_id text,
    external_author text,
    adapter text,
    external_message_id text,
    external_subject_key text,
    external_source_name text,
    status text DEFAULT 'pending'::text NOT NULL,
    delivered_at timestamp with time zone,
    run_id text,
    turn_number integer,
    session_id text,
    rewound_at timestamp with time zone,
    queue_order integer,
    CONSTRAINT task_inbox_channel_check CHECK (((adapter IS NOT NULL) OR (num_nonnulls(external_subject_key, external_source_name) = 0))),
    CONSTRAINT task_inbox_delivered_at_check CHECK (((status = ANY (ARRAY['delivered'::text, 'rewound'::text])) = (delivered_at IS NOT NULL))),
    CONSTRAINT task_inbox_origin_check CHECK ((origin = ANY (ARRAY['user'::text, 'task'::text, 'adapter'::text, 'system'::text, 'sandbox'::text, 'unknown'::text]))),
    CONSTRAINT task_inbox_rewound_at_check CHECK (((status = 'rewound'::text) = (rewound_at IS NOT NULL))),
    CONSTRAINT task_inbox_sender_check CHECK (
CASE origin
    WHEN 'user'::text THEN ((sent_by IS NOT NULL) AND (num_nonnulls(source_task_id, adapter, external_message_id, external_sender_id, external_author) = 0))
    WHEN 'task'::text THEN ((source_task_id IS NOT NULL) AND (num_nonnulls(adapter, external_message_id, external_sender_id, external_author) = 0))
    WHEN 'adapter'::text THEN ((adapter IS NOT NULL) AND (external_message_id IS NOT NULL) AND (source_task_id IS NULL))
    WHEN 'system'::text THEN (num_nonnulls(sent_by, source_task_id, adapter, external_message_id, external_sender_id, external_author) = 0)
    WHEN 'sandbox'::text THEN (num_nonnulls(sent_by, source_task_id, adapter, external_message_id, external_sender_id, external_author) = 0)
    WHEN 'unknown'::text THEN (status = ANY (ARRAY['delivered'::text, 'rewound'::text]))
    ELSE NULL::boolean
END),
    CONSTRAINT task_inbox_stamp_check CHECK (((status = ANY (ARRAY['delivered'::text, 'rewound'::text])) OR (num_nonnulls(run_id, turn_number, session_id) = 0))),
    CONSTRAINT task_inbox_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'delivered'::text, 'cancelled'::text, 'rewound'::text])))
);

CREATE TABLE task_listing_changes (
    "position" bigint NOT NULL,
    task_id text NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE task_listing_position (
    id boolean DEFAULT true NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL,
    CONSTRAINT task_listing_position_is_one_row CHECK (id)
);

CREATE TABLE task_merge_backs (
    target_task_id text NOT NULL,
    source_task_id text NOT NULL,
    through_turn integer NOT NULL,
    summary text NOT NULL,
    pending boolean DEFAULT true NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT task_merge_backs_through_turn_check CHECK ((through_turn > 0))
);

CREATE TABLE task_outbound_events (
    external_event_id text NOT NULL,
    task_id text NOT NULL,
    provider text NOT NULL,
    source_name text,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE task_read_receipts (
    user_id text NOT NULL,
    task_id text NOT NULL,
    last_read_at timestamp with time zone NOT NULL
);

CREATE TABLE task_relations (
    id text NOT NULL,
    from_task_id text NOT NULL,
    to_task_id text NOT NULL,
    relation_type text NOT NULL,
    context jsonb DEFAULT '{}'::jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE task_repositories (
    task_id text NOT NULL,
    repository_id text NOT NULL,
    source text NOT NULL,
    added_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT task_repositories_source_check CHECK ((source = ANY (ARRAY['workspace'::text, 'cloned'::text])))
);

CREATE TABLE task_rewound_turns (
    task_id text NOT NULL,
    turn_number integer NOT NULL,
    run_id text,
    status text NOT NULL,
    started_at timestamp with time zone NOT NULL,
    ended_at timestamp with time zone,
    rewound_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE task_tags (
    id integer NOT NULL,
    task_id text NOT NULL,
    tag_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE SEQUENCE task_tags_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE task_tags_id_seq OWNED BY task_tags.id;

CREATE TABLE task_turns (
    task_id text NOT NULL,
    turn_number integer NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    agent_profile text,
    started_at timestamp with time zone DEFAULT now() NOT NULL,
    ended_at timestamp with time zone,
    run_id text,
    recovery_attempts integer DEFAULT 0 NOT NULL,
    metrics jsonb,
    agent_turn_id text,
    triggered_by_task_id text,
    trigger_decided boolean DEFAULT false NOT NULL,
    interrupted_at timestamp with time zone,
    stop_unresponsive_at timestamp with time zone,
    CONSTRAINT task_turns_trigger_decided_check CHECK ((trigger_decided OR (triggered_by_task_id IS NULL)))
);

CREATE TABLE tasks (
    id integer NOT NULL,
    task_id text NOT NULL,
    short_id text,
    created_at text NOT NULL,
    last_message_at text NOT NULL,
    agent_profile text NOT NULL,
    repository_id text,
    status text NOT NULL,
    total_cost_usd double precision NOT NULL,
    template_id text,
    name text,
    description text,
    branch text,
    created_by text,
    visibility text DEFAULT 'private'::text NOT NULL,
    created_at_db timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    append_system_prompt text,
    headless boolean DEFAULT false NOT NULL,
    ttl_minutes integer DEFAULT 15 NOT NULL,
    owner_user_id text,
    close_after_settled_days integer DEFAULT 7 NOT NULL,
    loop_id text,
    workspace_id text,
    model text,
    closed_at timestamp with time zone,
    skills text[] DEFAULT '{}'::text[] NOT NULL,
    cold_after_hours integer,
    sandbox_id text,
    checkout_mode text DEFAULT 'shared'::text,
    run_as_user_id text,
    memory_preload text[] DEFAULT '{}'::text[] NOT NULL,
    settle_after_hours integer,
    auto_settled_at timestamp with time zone,
    reporter_user_id text,
    agent_properties jsonb DEFAULT '{}'::jsonb NOT NULL,
    CONSTRAINT tasks_checkout_mode_check CHECK (((checkout_mode IS NULL) OR (checkout_mode = ANY (ARRAY['shared'::text, 'isolated'::text]))))
);

CREATE SEQUENCE tasks_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE tasks_id_seq OWNED BY tasks.id;

CREATE TABLE terminal_session_changes (
    "position" bigint NOT NULL,
    origin text NOT NULL,
    task_id text NOT NULL,
    terminal_id text NOT NULL,
    summary jsonb NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE terminal_session_position (
    id boolean DEFAULT true NOT NULL,
    "position" bigint DEFAULT 0 NOT NULL,
    CONSTRAINT terminal_session_position_is_one_row CHECK (id)
);

CREATE TABLE trajectory_events (
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    seq bigint NOT NULL,
    parent_event_id uuid,
    kind text NOT NULL,
    harness_item_id text,
    harness_turn_id text,
    turn_number integer,
    payload text NOT NULL,
    content jsonb NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    ord bigint NOT NULL
);

CREATE SEQUENCE trajectory_events_ord_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER SEQUENCE trajectory_events_ord_seq OWNED BY trajectory_events.ord;

CREATE TABLE trajectory_sessions (
    id uuid NOT NULL,
    task_id text NOT NULL,
    harness text NOT NULL,
    harness_session_id text NOT NULL,
    forked_from_event_id uuid,
    last_event_id uuid,
    last_seq bigint DEFAULT '-1'::integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    parent_event_id uuid,
    transcoded_from_event_id uuid,
    own_from_seq bigint,
    CONSTRAINT trajectory_sessions_harness_check CHECK ((harness = ANY (ARRAY['claude-code'::text, 'codex'::text, 'pi'::text, 'mini-swe'::text, 'acp'::text]))),
    CONSTRAINT trajectory_sessions_subagent_lineage_check CHECK (((parent_event_id IS NULL) OR (num_nonnulls(transcoded_from_event_id, forked_from_event_id) = 0)))
);

CREATE TABLE user_credentials (
    user_id uuid NOT NULL,
    username text NOT NULL,
    password_hash text NOT NULL,
    must_change boolean DEFAULT false NOT NULL,
    failed_attempts integer DEFAULT 0 NOT NULL,
    last_login_at timestamp with time zone,
    password_changed_at timestamp with time zone DEFAULT now() NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE user_provider_ids (
    user_id uuid NOT NULL,
    provider text NOT NULL,
    provider_user_id text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE users (
    id uuid NOT NULL,
    login text NOT NULL,
    email text,
    name text,
    avatar_url text,
    provider text NOT NULL,
    role text DEFAULT 'user'::text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    last_login_at timestamp with time zone NOT NULL,
    settings jsonb DEFAULT '{}'::jsonb NOT NULL,
    is_bot boolean DEFAULT false NOT NULL,
    provider_user_id text,
    email_normalized text GENERATED ALWAYS AS (lower(btrim(email))) STORED,
    status text DEFAULT 'active'::text NOT NULL,
    CONSTRAINT users_human_email_required CHECK ((is_bot OR (email IS NOT NULL))),
    CONSTRAINT users_status_check CHECK ((status = ANY (ARRAY['active'::text, 'disabled'::text])))
);

CREATE TABLE workspace_credentials (
    id text NOT NULL,
    workspace_id text NOT NULL,
    provider text NOT NULL,
    value_ciphertext bytea NOT NULL,
    encryption_nonce bytea NOT NULL,
    encrypted_data_key bytea NOT NULL,
    kms_key_id text NOT NULL,
    encryption_version integer DEFAULT 1 NOT NULL,
    created_by text NOT NULL,
    updated_by text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT workspace_credentials_encryption_version_chk CHECK ((encryption_version > 0)),
    CONSTRAINT workspace_credentials_provider_chk CHECK ((provider = ANY (ARRAY['github'::text, 'gitlab'::text, 'gitness'::text, 'openai'::text, 'anthropic'::text])))
);

CREATE TABLE workspace_listing_changes (
    "position" bigint NOT NULL,
    workspace_id text NOT NULL,
    recorded_at timestamp with time zone DEFAULT now() NOT NULL
);

CREATE TABLE workspace_repos (
    id text NOT NULL,
    workspace_id text NOT NULL,
    repository_id text NOT NULL,
    branch text,
    mount_name text,
    is_primary boolean DEFAULT false NOT NULL,
    "position" integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now()
);

CREATE TABLE workspaces (
    id text NOT NULL,
    name text NOT NULL,
    description text,
    kind text DEFAULT 'single'::text NOT NULL,
    docker_image text,
    docker_access boolean,
    sandbox_service_account text,
    system_prompt text,
    setup_commands jsonb,
    env_vars jsonb,
    resources jsonb,
    scope text DEFAULT 'user'::text NOT NULL,
    owner_user_id text,
    created_by text,
    deleted boolean,
    created_at timestamp with time zone DEFAULT now(),
    updated_at timestamp with time zone DEFAULT now(),
    source text DEFAULT 'manual'::text NOT NULL,
    source_repository_id text,
    source_key text,
    network_profile text,
    source_config_path text,
    scripts jsonb DEFAULT '[]'::jsonb,
    run_as_user_id text,
    icon jsonb,
    default_model text,
    default_effort text,
    node_selector jsonb,
    language_servers jsonb,
    services jsonb,
    linear_team text,
    CONSTRAINT workspaces_owner_scope_chk CHECK ((((scope = 'user'::text) AND (owner_user_id IS NOT NULL)) OR ((scope = 'global'::text) AND (owner_user_id IS NULL))))
);

ALTER TABLE ONLY feedback ALTER COLUMN id SET DEFAULT nextval('feedback_id_seq'::regclass);

ALTER TABLE ONLY inbound_message_buffer ALTER COLUMN seq SET DEFAULT nextval('inbound_message_buffer_seq_seq'::regclass);

ALTER TABLE ONLY memory_events ALTER COLUMN event_id SET DEFAULT nextval('memory_events_event_id_seq'::regclass);

ALTER TABLE ONLY provider_tokens ALTER COLUMN id SET DEFAULT nextval('provider_tokens_id_seq'::regclass);

ALTER TABLE ONLY tags ALTER COLUMN id SET DEFAULT nextval('tags_id_seq'::regclass);

ALTER TABLE ONLY task_events ALTER COLUMN id SET DEFAULT nextval('task_events_id_seq'::regclass);

ALTER TABLE ONLY task_tags ALTER COLUMN id SET DEFAULT nextval('task_tags_id_seq'::regclass);

ALTER TABLE ONLY tasks ALTER COLUMN id SET DEFAULT nextval('tasks_id_seq'::regclass);

ALTER TABLE ONLY trajectory_events ALTER COLUMN ord SET DEFAULT nextval('trajectory_events_ord_seq'::regclass);

ALTER TABLE ONLY adapter_app_audit_events
    ADD CONSTRAINT adapter_app_audit_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY adapter_app_migrations
    ADD CONSTRAINT adapter_app_migrations_pkey PRIMARY KEY (name);

ALTER TABLE ONLY adapter_app_secrets
    ADD CONSTRAINT adapter_app_secrets_pkey PRIMARY KEY (adapter_app_id, name);

ALTER TABLE ONLY adapter_apps
    ADD CONSTRAINT adapter_apps_pkey PRIMARY KEY (id);

ALTER TABLE ONLY adapter_connection_events
    ADD CONSTRAINT adapter_connection_events_pkey PRIMARY KEY (connection_id, event_type, subject_key);

ALTER TABLE ONLY adapter_connections
    ADD CONSTRAINT adapter_connections_pkey PRIMARY KEY (id);

ALTER TABLE ONLY adapter_delivery
    ADD CONSTRAINT adapter_delivery_pkey PRIMARY KEY (id);

ALTER TABLE ONLY adapter_event_receipts
    ADD CONSTRAINT adapter_event_receipts_pkey PRIMARY KEY (adapter_kind, external_event_id, task_id);

ALTER TABLE ONLY adapter_task_creation_claims
    ADD CONSTRAINT adapter_task_creation_claims_pkey PRIMARY KEY (loop_id, subject_kind, subject_key);

ALTER TABLE ONLY api_keys
    ADD CONSTRAINT api_keys_pkey PRIMARY KEY (id);

ALTER TABLE ONLY auth_audit_events
    ADD CONSTRAINT auth_audit_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY auth_rate_limits
    ADD CONSTRAINT auth_rate_limits_pkey PRIMARY KEY (bucket, subject_hash, window_started_at);

ALTER TABLE ONLY auth_sessions
    ADD CONSTRAINT auth_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY auth_sessions
    ADD CONSTRAINT auth_sessions_token_hash_key UNIQUE (token_hash);

ALTER TABLE ONLY auth_transactions
    ADD CONSTRAINT auth_transactions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY auth_transactions
    ADD CONSTRAINT auth_transactions_state_hash_key UNIQUE (state_hash);

ALTER TABLE ONLY collection_record_changes
    ADD CONSTRAINT collection_record_changes_pkey PRIMARY KEY (collection_id, seq);

ALTER TABLE ONLY collection_records
    ADD CONSTRAINT collection_records_pkey PRIMARY KEY (collection_id, key);

ALTER TABLE ONLY conversation_activities
    ADD CONSTRAINT conversation_activities_pkey PRIMARY KEY (activity_id);

ALTER TABLE ONLY conversation_frozen
    ADD CONSTRAINT conversation_frozen_pkey PRIMARY KEY (task_id);

ALTER TABLE ONLY conversation_messages
    ADD CONSTRAINT conversation_messages_pkey PRIMARY KEY (uuid);

ALTER TABLE ONLY conversation_usage
    ADD CONSTRAINT conversation_usage_message_uuid_key UNIQUE (message_uuid);

ALTER TABLE ONLY conversation_usage
    ADD CONSTRAINT conversation_usage_task_id_trajectory_event_id_key UNIQUE (task_id, trajectory_event_id);

ALTER TABLE ONLY device_pairing_codes
    ADD CONSTRAINT device_pairing_codes_pkey PRIMARY KEY (id);

ALTER TABLE ONLY feedback
    ADD CONSTRAINT feedback_feedback_id_key UNIQUE (feedback_id);

ALTER TABLE ONLY feedback
    ADD CONSTRAINT feedback_pkey PRIMARY KEY (id);

ALTER TABLE ONLY file_metadata
    ADD CONSTRAINT file_metadata_pkey PRIMARY KEY (file_id);

ALTER TABLE ONLY forgejo_instances
    ADD CONSTRAINT forgejo_instances_pkey PRIMARY KEY (id);

ALTER TABLE ONLY harness_cooldowns
    ADD CONSTRAINT harness_cooldowns_pkey PRIMARY KEY (user_id, harness);

ALTER TABLE ONLY inbound_message_buffer
    ADD CONSTRAINT inbound_message_buffer_pkey PRIMARY KEY (id);

ALTER TABLE ONLY loop_configs
    ADD CONSTRAINT loop_configs_pkey PRIMARY KEY (loop_id);

ALTER TABLE ONLY loops
    ADD CONSTRAINT loops_pkey PRIMARY KEY (id);

ALTER TABLE ONLY memory_entries
    ADD CONSTRAINT memory_entries_pkey PRIMARY KEY (entry_id);

ALTER TABLE ONLY memory_entries
    ADD CONSTRAINT memory_entries_scope_kind_scope_id_key_key UNIQUE (scope_kind, scope_id, key);

ALTER TABLE ONLY memory_events
    ADD CONSTRAINT memory_events_pkey PRIMARY KEY (event_id);

ALTER TABLE ONLY memory_reads
    ADD CONSTRAINT memory_reads_pkey PRIMARY KEY (entry_id, task_id, version);

ALTER TABLE ONLY memory_seen
    ADD CONSTRAINT memory_seen_pkey PRIMARY KEY (task_id);

ALTER TABLE ONLY oidc_identities
    ADD CONSTRAINT oidc_identities_issuer_subject_key UNIQUE (issuer, subject);

ALTER TABLE ONLY oidc_identities
    ADD CONSTRAINT oidc_identities_pkey PRIMARY KEY (id);

ALTER TABLE ONLY oidc_identities
    ADD CONSTRAINT oidc_identities_user_id_issuer_key UNIQUE (user_id, issuer);

ALTER TABLE ONLY plugin_activations
    ADD CONSTRAINT plugin_activations_subject_key UNIQUE NULLS NOT DISTINCT (plugin_id, skill_name, user_id, workspace_id);

ALTER TABLE ONLY plugins
    ADD CONSTRAINT plugin_repos_name_key UNIQUE (name);

ALTER TABLE ONLY plugins
    ADD CONSTRAINT plugin_repos_pkey PRIMARY KEY (id);

ALTER TABLE ONLY plugin_skills
    ADD CONSTRAINT plugin_skills_pkey PRIMARY KEY (plugin_id, name);

ALTER TABLE ONLY preview_tab_changes
    ADD CONSTRAINT preview_tab_changes_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY preview_tab_position
    ADD CONSTRAINT preview_tab_position_pkey PRIMARY KEY (id);

ALTER TABLE ONLY provider_tokens
    ADD CONSTRAINT provider_tokens_pkey PRIMARY KEY (id);

ALTER TABLE ONLY provider_tokens
    ADD CONSTRAINT provider_tokens_scope_uq UNIQUE NULLS NOT DISTINCT (user_id, provider);

ALTER TABLE ONLY record_collections
    ADD CONSTRAINT record_collections_artifact_id_key UNIQUE (artifact_id);

ALTER TABLE ONLY record_collections
    ADD CONSTRAINT record_collections_pkey PRIMARY KEY (collection_id);

ALTER TABLE ONLY record_collections
    ADD CONSTRAINT record_collections_scope_kind_scope_id_name_key UNIQUE (scope_kind, scope_id, name);

ALTER TABLE ONLY repositories
    ADD CONSTRAINT repositories_pkey PRIMARY KEY (id);

ALTER TABLE ONLY sandbox_unreported_commands
    ADD CONSTRAINT sandbox_unreported_commands_pkey PRIMARY KEY (task_id);

ALTER TABLE ONLY sandboxes
    ADD CONSTRAINT sandboxes_pkey PRIMARY KEY (sandbox_id);

ALTER TABLE ONLY schedules
    ADD CONSTRAINT schedules_pkey PRIMARY KEY (loop_id);

ALTER TABLE ONLY secret_audit_events
    ADD CONSTRAINT secret_audit_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY secrets
    ADD CONSTRAINT secrets_pkey PRIMARY KEY (id);

ALTER TABLE ONLY subscriptions
    ADD CONSTRAINT subscriptions_pkey PRIMARY KEY (loop_id);

ALTER TABLE ONLY tags
    ADD CONSTRAINT tags_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_artifact_versions
    ADD CONSTRAINT task_artifact_versions_pkey PRIMARY KEY (artifact_id, version);

ALTER TABLE ONLY task_artifacts
    ADD CONSTRAINT task_artifacts_pkey PRIMARY KEY (artifact_id);

ALTER TABLE ONLY task_artifacts
    ADD CONSTRAINT task_artifacts_task_id_slug_key UNIQUE (task_id, slug);

ALTER TABLE ONLY task_bindings
    ADD CONSTRAINT task_bindings_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_conversation_changes
    ADD CONSTRAINT task_conversation_changes_pkey PRIMARY KEY (task_id, "position");

ALTER TABLE ONLY task_conversation_position
    ADD CONSTRAINT task_conversation_position_pkey PRIMARY KEY (task_id);

ALTER TABLE ONLY task_event_changes
    ADD CONSTRAINT task_event_changes_event_id_key UNIQUE (event_id);

ALTER TABLE ONLY task_event_changes
    ADD CONSTRAINT task_event_changes_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY task_event_dispatch_queue
    ADD CONSTRAINT task_event_dispatch_queue_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY task_event_position
    ADD CONSTRAINT task_event_position_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_events
    ADD CONSTRAINT task_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_follows
    ADD CONSTRAINT task_follows_pkey PRIMARY KEY (user_id, task_id);

ALTER TABLE ONLY task_forks
    ADD CONSTRAINT task_forks_pkey PRIMARY KEY (task_id);

ALTER TABLE ONLY task_inbox
    ADD CONSTRAINT task_inbox_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_listing_changes
    ADD CONSTRAINT task_listing_changes_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY task_listing_position
    ADD CONSTRAINT task_listing_position_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_merge_backs
    ADD CONSTRAINT task_merge_backs_pkey PRIMARY KEY (target_task_id, source_task_id);

ALTER TABLE ONLY task_outbound_events
    ADD CONSTRAINT task_outbound_events_pkey PRIMARY KEY (external_event_id);

ALTER TABLE ONLY task_read_receipts
    ADD CONSTRAINT task_read_receipts_pkey PRIMARY KEY (user_id, task_id);

ALTER TABLE ONLY task_relations
    ADD CONSTRAINT task_relations_from_task_id_to_task_id_relation_type_key UNIQUE (from_task_id, to_task_id, relation_type);

ALTER TABLE ONLY task_relations
    ADD CONSTRAINT task_relations_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_repositories
    ADD CONSTRAINT task_repositories_pkey PRIMARY KEY (task_id, repository_id);

ALTER TABLE ONLY task_rewound_turns
    ADD CONSTRAINT task_rewound_turns_pkey PRIMARY KEY (task_id, turn_number);

ALTER TABLE ONLY task_tags
    ADD CONSTRAINT task_tags_pkey PRIMARY KEY (id);

ALTER TABLE ONLY task_turns
    ADD CONSTRAINT task_turns_pkey PRIMARY KEY (task_id, turn_number);

ALTER TABLE ONLY terminal_session_changes
    ADD CONSTRAINT terminal_session_changes_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY terminal_session_position
    ADD CONSTRAINT terminal_session_position_pkey PRIMARY KEY (id);

ALTER TABLE ONLY trajectory_events
    ADD CONSTRAINT trajectory_events_pkey PRIMARY KEY (id);

ALTER TABLE ONLY trajectory_events
    ADD CONSTRAINT trajectory_events_session_id_seq_key UNIQUE (session_id, seq);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_harness_session_id_key UNIQUE (harness_session_id);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_pkey PRIMARY KEY (id);

ALTER TABLE ONLY user_credentials
    ADD CONSTRAINT user_credentials_pkey PRIMARY KEY (user_id);

ALTER TABLE ONLY user_provider_ids
    ADD CONSTRAINT user_provider_ids_pkey PRIMARY KEY (user_id, provider);

ALTER TABLE ONLY users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);

ALTER TABLE ONLY workspace_credentials
    ADD CONSTRAINT workspace_credentials_pkey PRIMARY KEY (id);

ALTER TABLE ONLY workspace_listing_changes
    ADD CONSTRAINT workspace_listing_changes_pkey PRIMARY KEY ("position");

ALTER TABLE ONLY workspace_repos
    ADD CONSTRAINT workspace_repos_pkey PRIMARY KEY (id);

ALTER TABLE ONLY workspaces
    ADD CONSTRAINT workspaces_pkey PRIMARY KEY (id);

CREATE INDEX adapter_app_audit_events_actor_idx ON adapter_app_audit_events USING btree (actor_id, created_at);

CREATE INDEX adapter_app_audit_events_app_idx ON adapter_app_audit_events USING btree (adapter_kind, app_key, created_at);

CREATE INDEX adapter_apps_kind_idx ON adapter_apps USING btree (adapter_kind, enabled, app_key);

CREATE UNIQUE INDEX adapter_apps_kind_key_uq ON adapter_apps USING btree (adapter_kind, app_key);

CREATE INDEX adapter_connection_events_event_received_idx ON adapter_connection_events USING btree (event_type, received_at);

CREATE UNIQUE INDEX adapter_connections_adapter_account_idx ON adapter_connections USING btree (adapter_kind, external_account_id);

CREATE INDEX adapter_connections_adapter_idx ON adapter_connections USING btree (adapter_kind);

CREATE INDEX adapter_delivery_claimed_idx ON adapter_delivery USING btree (claimed_at) WHERE (status = 'claimed'::text);

CREATE INDEX adapter_delivery_due_idx ON adapter_delivery USING btree (next_attempt_at) WHERE (status = 'pending'::text);

CREATE UNIQUE INDEX adapter_delivery_event_idx ON adapter_delivery USING btree (adapter, kind, event_id);

CREATE INDEX adapter_delivery_finished_idx ON adapter_delivery USING btree (updated_at) WHERE (status = ANY (ARRAY['succeeded'::text, 'failed'::text]));

CREATE INDEX adapter_delivery_unfinished_idx ON adapter_delivery USING btree (id) WHERE (status = ANY (ARRAY['pending'::text, 'claimed'::text]));

CREATE INDEX adapter_event_receipts_subject_idx ON adapter_event_receipts USING btree (adapter_kind, subject_kind, subject_key);

CREATE INDEX auth_audit_events_user_time_idx ON auth_audit_events USING btree (user_id, occurred_at DESC);

CREATE INDEX auth_rate_limits_window_idx ON auth_rate_limits USING btree (window_started_at);

CREATE INDEX auth_sessions_expiry_idx ON auth_sessions USING btree (absolute_expires_at) WHERE (revoked_at IS NULL);

CREATE INDEX auth_sessions_user_active_idx ON auth_sessions USING btree (user_id) WHERE (revoked_at IS NULL);

CREATE INDEX auth_transactions_expiry_idx ON auth_transactions USING btree (expires_at);

CREATE INDEX collection_records_updated_idx ON collection_records USING btree (collection_id, updated_at);

CREATE INDEX conversation_activities_message_idx ON conversation_activities USING btree (task_id, message_uuid) WHERE (message_uuid IS NOT NULL);

CREATE INDEX conversation_activities_parent_idx ON conversation_activities USING btree (task_id, parent_tool_use_id) WHERE (parent_tool_use_id IS NOT NULL);

CREATE INDEX conversation_activities_task_event_idx ON conversation_activities USING btree (task_event_id) WHERE (task_event_id IS NOT NULL);

CREATE INDEX conversation_activities_task_idx ON conversation_activities USING btree (task_id, "position");

CREATE UNIQUE INDEX conversation_activities_tool_use_idx ON conversation_activities USING btree (task_id, tool_use_id) WHERE (tool_use_id IS NOT NULL);

CREATE INDEX conversation_activities_top_level_call_idx ON conversation_activities USING btree (task_id, "position") INCLUDE (message_uuid) WHERE ((parent_tool_use_id IS NULL) AND (kind = 'tool_call'::text));

CREATE INDEX conversation_messages_event_idx ON conversation_messages USING btree (trajectory_event_id);

CREATE INDEX conversation_messages_inbox_idx ON conversation_messages USING btree (inbox_id);

CREATE INDEX conversation_messages_latest_user_idx ON conversation_messages USING btree (task_id, created_at DESC) WHERE (author = 'user'::text);

CREATE INDEX conversation_messages_parent_idx ON conversation_messages USING btree (task_id, parent_tool_use_id, "position") WHERE (parent_tool_use_id IS NOT NULL);

CREATE INDEX conversation_messages_task_event_idx ON conversation_messages USING btree (task_event_id) WHERE (task_event_id IS NOT NULL);

CREATE UNIQUE INDEX conversation_messages_task_position_idx ON conversation_messages USING btree (task_id, "position") WHERE (parent_tool_use_id IS NULL);

CREATE INDEX conversation_messages_task_turn_idx ON conversation_messages USING btree (task_id, turn_number);

CREATE INDEX conversation_messages_turn_agent_model_idx ON conversation_messages USING btree (task_id, turn_number, "position" DESC) INCLUDE (model) WHERE ((author = 'agent'::text) AND (model IS NOT NULL));

CREATE INDEX conversation_messages_turn_inbox_idx ON conversation_messages USING btree (task_id, turn_number, "position") INCLUDE (inbox_id) WHERE (inbox_id IS NOT NULL);

CREATE INDEX conversation_usage_task_idx ON conversation_usage USING btree (task_id);

CREATE UNIQUE INDEX device_pairing_codes_code_hash_uq ON device_pairing_codes USING btree (code_hash);

CREATE INDEX device_pairing_codes_expires_at_idx ON device_pairing_codes USING btree (expires_at);

CREATE INDEX feedback_category_idx ON feedback USING btree (category) WHERE (category IS NOT NULL);

CREATE INDEX feedback_message_uuid_idx ON feedback USING btree (message_uuid) WHERE (message_uuid IS NOT NULL);

CREATE INDEX feedback_status_idx ON feedback USING btree (status);

CREATE INDEX feedback_task_id_idx ON feedback USING btree (task_id);

CREATE INDEX feedback_tool_use_id_idx ON feedback USING btree (tool_use_id) WHERE (tool_use_id IS NOT NULL);

CREATE INDEX file_metadata_scope_owner_idx ON file_metadata USING btree (scope, owner_user_id);

CREATE UNIQUE INDEX forgejo_instances_host_uq ON forgejo_instances USING btree (host);

CREATE INDEX idx_plugin_activations_global ON plugin_activations USING btree (plugin_id) WHERE ((user_id IS NULL) AND (workspace_id IS NULL));

CREATE INDEX idx_plugin_activations_user ON plugin_activations USING btree (user_id) WHERE (user_id IS NOT NULL);

CREATE INDEX idx_plugin_activations_workspace ON plugin_activations USING btree (workspace_id) WHERE (workspace_id IS NOT NULL);

CREATE INDEX idx_provider_tokens_codex_healthy_all_scopes ON provider_tokens USING btree (scope, user_id) WHERE ((provider = 'codex'::text) AND ((metadata ->> 'refresh_failure'::text) IS NULL));

CREATE UNIQUE INDEX idx_task_events_event_id ON task_events USING btree (event_id);

CREATE INDEX idx_task_events_task_id ON task_events USING btree (task_id, id);

CREATE INDEX idx_task_relations_from ON task_relations USING btree (from_task_id, relation_type);

CREATE INDEX idx_task_relations_to ON task_relations USING btree (to_task_id, relation_type);

CREATE INDEX idx_task_turns_open ON task_turns USING btree (started_at) WHERE (status = ANY (ARRAY['active'::text, 'interrupted'::text]));

CREATE UNIQUE INDEX idx_task_turns_run ON task_turns USING btree (task_id, run_id) WHERE (run_id IS NOT NULL);

CREATE INDEX idx_users_is_bot ON users USING btree (is_bot) WHERE is_bot;

CREATE INDEX inbound_message_buffer_binding_idx ON inbound_message_buffer USING btree (binding_id, seq);

CREATE INDEX inbound_message_buffer_due_idx ON inbound_message_buffer USING btree (flush_at) WHERE (claimed_at IS NULL);

CREATE UNIQUE INDEX loops_git_provenance_idx ON loops USING btree (source_repository_id, source_key) WHERE ((source_repository_id IS NOT NULL) AND (source_key IS NOT NULL));

CREATE INDEX loops_kind_idx ON loops USING btree (kind);

CREATE INDEX loops_source_repository_idx ON loops USING btree (source_repository_id);

CREATE INDEX memory_entries_key_prefix_idx ON memory_entries USING btree (scope_kind, scope_id, key text_pattern_ops);

CREATE INDEX memory_entries_search_idx ON memory_entries USING gin (search);

CREATE INDEX memory_events_entry_idx ON memory_events USING btree (entry_id, event_id DESC);

CREATE UNIQUE INDEX repositories_active_owner_remote_url_uq ON repositories USING btree (scope, owner_user_id, lower(regexp_replace(regexp_replace(regexp_replace(btrim(remote_url), '/+$'::text, ''::text), '\.git$'::text, ''::text), '/+$'::text, ''::text))) NULLS NOT DISTINCT WHERE ((remote_url IS NOT NULL) AND (COALESCE(deleted, false) = false));

CREATE INDEX repositories_created_by_idx ON repositories USING btree (created_by);

CREATE INDEX repositories_scope_owner_idx ON repositories USING btree (scope, owner_user_id);

CREATE INDEX sandboxes_desired_running_idx ON sandboxes USING btree (sandbox_id) WHERE (desired_state = 'running'::text);

CREATE INDEX sandboxes_incomplete_operation_idx ON sandboxes USING btree (sandbox_id) WHERE (operation_generation IS NOT NULL);

CREATE INDEX sandboxes_pending_work_idx ON sandboxes USING btree (sandbox_id) WHERE (pending_work_at IS NOT NULL);

CREATE INDEX sandboxes_reclaimable_idx ON sandboxes USING btree (desired_state_updated_at) WHERE ((desired_state = 'stopped'::text) AND (storage_reclaimed_at IS NULL));

CREATE INDEX schedules_due_idx ON schedules USING btree (next_fire_at);

CREATE INDEX secret_audit_events_actor_idx ON secret_audit_events USING btree (actor_id, created_at);

CREATE INDEX secret_audit_events_secret_idx ON secret_audit_events USING btree (secret_id, created_at);

CREATE UNIQUE INDEX secrets_scope_key_uq ON secrets USING btree (scope, COALESCE(owner_user_id, ''::text), key);

CREATE INDEX secrets_scope_owner_idx ON secrets USING btree (scope, owner_user_id);

CREATE INDEX subscriptions_adapter_source_idx ON subscriptions USING btree (adapter_kind, source_matcher);

CREATE INDEX subscriptions_connection_idx ON subscriptions USING btree (connection_id);

CREATE UNIQUE INDEX tags_name_idx ON tags USING btree (name);

CREATE UNIQUE INDEX tags_tag_id_idx ON tags USING btree (tag_id);

CREATE INDEX task_bindings_active_idx ON task_bindings USING btree (active);

CREATE INDEX task_bindings_subject_idx ON task_bindings USING btree (adapter_kind, subject_kind, subject_key);

CREATE INDEX task_bindings_task_idx ON task_bindings USING btree (task_id);

CREATE UNIQUE INDEX task_bindings_task_subject_idx ON task_bindings USING btree (task_id, adapter_kind, subject_kind, subject_key);

CREATE INDEX task_event_dispatch_queue_claimable ON task_event_dispatch_queue USING btree (claimed_at NULLS FIRST, "position");

CREATE INDEX task_events_turn_error_idx ON task_events USING btree (task_id, turn_number, id) WHERE (sub_type = 'error'::text);

CREATE INDEX task_follows_task_idx ON task_follows USING btree (task_id);

CREATE INDEX task_follows_user_idx ON task_follows USING btree (user_id);

CREATE UNIQUE INDEX task_inbox_external_message_idx ON task_inbox USING btree (task_id, adapter, external_message_id) WHERE (external_message_id IS NOT NULL);

CREATE INDEX task_inbox_latest_queued_idx ON task_inbox USING btree (task_id, created_at DESC) WHERE (status = ANY (ARRAY['pending'::text, 'delivered'::text]));

CREATE INDEX task_inbox_pending_idx ON task_inbox USING btree (task_id, created_at) WHERE (status = 'pending'::text);

CREATE INDEX task_inbox_sent_by_idx ON task_inbox USING btree (sent_by);

CREATE INDEX task_inbox_turn_idx ON task_inbox USING btree (task_id, turn_number);

CREATE INDEX task_outbound_events_task_idx ON task_outbound_events USING btree (task_id);

CREATE INDEX task_read_receipts_task_idx ON task_read_receipts USING btree (task_id);

CREATE INDEX task_repositories_repository_idx ON task_repositories USING btree (repository_id);

CREATE INDEX task_tags_tag_id_idx ON task_tags USING btree (tag_id);

CREATE UNIQUE INDEX task_tags_task_id_tag_id_idx ON task_tags USING btree (task_id, tag_id);

CREATE INDEX tasks_created_by_idx ON tasks USING btree (created_by);

CREATE INDEX tasks_last_message_idx ON tasks USING btree (last_message_at);

CREATE INDEX tasks_listing_pick_reporter_idx ON tasks USING btree (status, updated_at DESC NULLS LAST, task_id) INCLUDE (visibility, owner_user_id, created_by, reporter_user_id);

CREATE INDEX tasks_loop_id_idx ON tasks USING btree (loop_id) WHERE (loop_id IS NOT NULL);

CREATE INDEX tasks_repository_idx ON tasks USING btree (repository_id);

CREATE INDEX tasks_sandbox_id_idx ON tasks USING btree (sandbox_id);

CREATE UNIQUE INDEX tasks_short_id_idx ON tasks USING btree (short_id);

CREATE INDEX tasks_status_idx ON tasks USING btree (status);

CREATE UNIQUE INDEX tasks_task_id_idx ON tasks USING btree (task_id);

CREATE INDEX tasks_updated_at_idx ON tasks USING btree (updated_at);

CREATE INDEX tasks_visibility_idx ON tasks USING btree (visibility);

CREATE INDEX tasks_workspace_idx ON tasks USING btree (workspace_id);

CREATE INDEX trajectory_events_session_item_idx ON trajectory_events USING btree (session_id, harness_item_id);

CREATE INDEX trajectory_events_session_turn_idx ON trajectory_events USING btree (session_id, harness_turn_id);

CREATE INDEX trajectory_events_session_turn_seq_idx ON trajectory_events USING btree (session_id, turn_number, seq);

CREATE INDEX trajectory_sessions_task_id_created_at_idx ON trajectory_sessions USING btree (task_id, created_at);

CREATE INDEX trajectory_sessions_task_transcoded_idx ON trajectory_sessions USING btree (task_id, transcoded_from_event_id);

CREATE UNIQUE INDEX user_credentials_username_uq ON user_credentials USING btree (lower(username));

CREATE UNIQUE INDEX user_provider_ids_lookup_idx ON user_provider_ids USING btree (provider, provider_user_id);

CREATE UNIQUE INDEX users_human_email_normalized_uq ON users USING btree (email_normalized) WHERE (NOT is_bot);

CREATE UNIQUE INDEX workspace_credentials_workspace_provider_uq ON workspace_credentials USING btree (workspace_id, provider);

CREATE UNIQUE INDEX workspace_repos_mount_unique ON workspace_repos USING btree (workspace_id, mount_name) WHERE (mount_name IS NOT NULL);

CREATE UNIQUE INDEX workspace_repos_one_primary ON workspace_repos USING btree (workspace_id) WHERE is_primary;

CREATE INDEX workspace_repos_repository_idx ON workspace_repos USING btree (repository_id);

CREATE INDEX workspace_repos_workspace_idx ON workspace_repos USING btree (workspace_id);

CREATE INDEX workspaces_created_by_idx ON workspaces USING btree (created_by);

CREATE UNIQUE INDEX workspaces_git_source_unique ON workspaces USING btree (source_repository_id, source_key) WHERE (source = 'git'::text);

CREATE INDEX workspaces_run_as_user_id_idx ON workspaces USING btree (run_as_user_id) WHERE (run_as_user_id IS NOT NULL);

CREATE INDEX workspaces_scope_idx ON workspaces USING btree (scope);

CREATE INDEX workspaces_source_repository_idx ON workspaces USING btree (source_repository_id) WHERE (source_repository_id IS NOT NULL);

CREATE TRIGGER conversation_messages_agent_event BEFORE INSERT OR UPDATE ON conversation_messages FOR EACH ROW EXECUTE FUNCTION conversation_messages_agent_event();

CREATE CONSTRAINT TRIGGER trajectory_sessions_fork_same_harness AFTER INSERT OR UPDATE OF forked_from_event_id, harness ON trajectory_sessions NOT DEFERRABLE INITIALLY IMMEDIATE FOR EACH ROW EXECUTE FUNCTION trajectory_sessions_fork_same_harness();

ALTER TABLE ONLY adapter_app_secrets
    ADD CONSTRAINT adapter_app_secrets_adapter_app_id_fkey FOREIGN KEY (adapter_app_id) REFERENCES adapter_apps(id) ON DELETE CASCADE;

ALTER TABLE ONLY adapter_connection_events
    ADD CONSTRAINT adapter_connection_events_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES adapter_connections(id) ON DELETE CASCADE;

ALTER TABLE ONLY auth_audit_events
    ADD CONSTRAINT auth_audit_events_auth_session_id_fkey FOREIGN KEY (auth_session_id) REFERENCES auth_sessions(id) ON DELETE SET NULL;

ALTER TABLE ONLY auth_audit_events
    ADD CONSTRAINT auth_audit_events_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL;

ALTER TABLE ONLY auth_sessions
    ADD CONSTRAINT auth_sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE ONLY collection_record_changes
    ADD CONSTRAINT collection_record_changes_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES record_collections(collection_id) ON DELETE CASCADE;

ALTER TABLE ONLY collection_records
    ADD CONSTRAINT collection_records_collection_id_fkey FOREIGN KEY (collection_id) REFERENCES record_collections(collection_id) ON DELETE CASCADE;

ALTER TABLE ONLY conversation_activities
    ADD CONSTRAINT conversation_activities_inbox_id_fkey FOREIGN KEY (inbox_id) REFERENCES task_inbox(id) ON DELETE CASCADE;

ALTER TABLE ONLY conversation_activities
    ADD CONSTRAINT conversation_activities_result_event_id_fkey FOREIGN KEY (result_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY conversation_activities
    ADD CONSTRAINT conversation_activities_trajectory_event_id_fkey FOREIGN KEY (trajectory_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY conversation_messages
    ADD CONSTRAINT conversation_messages_inbox_id_fkey FOREIGN KEY (inbox_id) REFERENCES task_inbox(id) ON DELETE CASCADE;

ALTER TABLE ONLY conversation_messages
    ADD CONSTRAINT conversation_messages_trajectory_event_id_fkey FOREIGN KEY (trajectory_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY conversation_usage
    ADD CONSTRAINT conversation_usage_trajectory_event_id_fkey FOREIGN KEY (trajectory_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY inbound_message_buffer
    ADD CONSTRAINT inbound_message_buffer_binding_id_fkey FOREIGN KEY (binding_id) REFERENCES task_bindings(id) ON DELETE CASCADE;

ALTER TABLE ONLY loop_configs
    ADD CONSTRAINT loop_configs_loop_id_fkey FOREIGN KEY (loop_id) REFERENCES loops(id) ON DELETE CASCADE;

ALTER TABLE ONLY memory_events
    ADD CONSTRAINT memory_events_entry_id_fkey FOREIGN KEY (entry_id) REFERENCES memory_entries(entry_id) ON DELETE CASCADE;

ALTER TABLE ONLY memory_reads
    ADD CONSTRAINT memory_reads_entry_id_fkey FOREIGN KEY (entry_id) REFERENCES memory_entries(entry_id) ON DELETE CASCADE;

ALTER TABLE ONLY oidc_identities
    ADD CONSTRAINT oidc_identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE ONLY plugin_activations
    ADD CONSTRAINT plugin_activations_plugin_id_fkey FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE;

ALTER TABLE ONLY plugin_activations
    ADD CONSTRAINT plugin_activations_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE ONLY plugin_activations
    ADD CONSTRAINT plugin_activations_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE;

ALTER TABLE ONLY plugin_skills
    ADD CONSTRAINT plugin_skills_plugin_id_fkey FOREIGN KEY (plugin_id) REFERENCES plugins(id) ON DELETE CASCADE;

ALTER TABLE ONLY record_collections
    ADD CONSTRAINT record_collections_artifact_id_fkey FOREIGN KEY (artifact_id) REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE;

ALTER TABLE ONLY sandbox_unreported_commands
    ADD CONSTRAINT sandbox_unreported_commands_task_id_fkey FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY sandboxes
    ADD CONSTRAINT sandboxes_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE RESTRICT;

ALTER TABLE ONLY schedules
    ADD CONSTRAINT schedules_loop_id_fkey FOREIGN KEY (loop_id) REFERENCES loops(id) ON DELETE CASCADE;

ALTER TABLE ONLY subscriptions
    ADD CONSTRAINT subscriptions_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES adapter_connections(id) ON DELETE CASCADE;

ALTER TABLE ONLY subscriptions
    ADD CONSTRAINT subscriptions_loop_id_fkey FOREIGN KEY (loop_id) REFERENCES loops(id) ON DELETE CASCADE;

ALTER TABLE ONLY task_artifact_versions
    ADD CONSTRAINT task_artifact_versions_artifact_id_fkey FOREIGN KEY (artifact_id) REFERENCES task_artifacts(artifact_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_artifacts
    ADD CONSTRAINT task_artifacts_task_id_fkey FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_bindings
    ADD CONSTRAINT task_bindings_connection_id_fkey FOREIGN KEY (connection_id) REFERENCES adapter_connections(id) ON DELETE CASCADE;

ALTER TABLE ONLY task_bindings
    ADD CONSTRAINT task_bindings_loop_id_fkey FOREIGN KEY (loop_id) REFERENCES loops(id) ON DELETE SET NULL;

ALTER TABLE ONLY task_forks
    ADD CONSTRAINT task_forks_source_event_id_fkey FOREIGN KEY (source_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY task_forks
    ADD CONSTRAINT task_forks_task_id_fkey FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_merge_backs
    ADD CONSTRAINT task_merge_backs_target_task_id_fkey FOREIGN KEY (target_task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_outbound_events
    ADD CONSTRAINT task_outbound_events_task_fk FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_relations
    ADD CONSTRAINT task_relations_from_task_id_fkey FOREIGN KEY (from_task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_relations
    ADD CONSTRAINT task_relations_to_task_id_fkey FOREIGN KEY (to_task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_repositories
    ADD CONSTRAINT task_repositories_repository_id_fkey FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE;

ALTER TABLE ONLY task_repositories
    ADD CONSTRAINT task_repositories_task_id_fkey FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY task_rewound_turns
    ADD CONSTRAINT task_rewound_turns_task_id_fkey FOREIGN KEY (task_id) REFERENCES tasks(task_id) ON DELETE CASCADE;

ALTER TABLE ONLY tasks
    ADD CONSTRAINT tasks_loop_id_fkey FOREIGN KEY (loop_id) REFERENCES loops(id) ON DELETE SET NULL;

ALTER TABLE ONLY tasks
    ADD CONSTRAINT tasks_sandbox_id_fkey FOREIGN KEY (sandbox_id) REFERENCES sandboxes(sandbox_id) ON DELETE SET NULL;

ALTER TABLE ONLY trajectory_events
    ADD CONSTRAINT trajectory_events_parent_event_id_fkey FOREIGN KEY (parent_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY trajectory_events
    ADD CONSTRAINT trajectory_events_session_id_fkey FOREIGN KEY (session_id) REFERENCES trajectory_sessions(id);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_forked_from_event_fkey FOREIGN KEY (forked_from_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_last_event_fkey FOREIGN KEY (last_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_parent_event_id_fkey FOREIGN KEY (parent_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_transcoded_from_event_id_fkey FOREIGN KEY (transcoded_from_event_id) REFERENCES trajectory_events(id);

ALTER TABLE ONLY user_credentials
    ADD CONSTRAINT user_credentials_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE ONLY user_provider_ids
    ADD CONSTRAINT user_provider_ids_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

ALTER TABLE ONLY workspace_credentials
    ADD CONSTRAINT workspace_credentials_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE;

ALTER TABLE ONLY workspace_repos
    ADD CONSTRAINT workspace_repos_workspace_id_fkey FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE;

-- Seed rows the old chain inserted.
INSERT INTO preview_tab_position VALUES (true, 0);
INSERT INTO task_event_position VALUES (true, 0);
INSERT INTO task_listing_position VALUES (true, 0);
INSERT INTO terminal_session_position VALUES (true, 0);
