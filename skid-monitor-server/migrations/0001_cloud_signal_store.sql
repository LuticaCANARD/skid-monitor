CREATE TABLE public.tenants (
    id UUID PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT tenants_slug_not_blank CHECK (btrim(slug) <> ''),
    CONSTRAINT tenants_display_name_not_blank CHECK (btrim(display_name) <> '')
);

CREATE TABLE public.agents (
    tenant_id UUID NOT NULL REFERENCES public.tenants(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL,
    display_name TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    enrolled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, agent_id),
    CONSTRAINT agents_id_size CHECK (
        octet_length(agent_id) BETWEEN 1 AND 255
    ),
    CONSTRAINT agents_id_no_control_characters CHECK (
        agent_id !~ '[[:cntrl:]]'
    ),
    CONSTRAINT agents_display_name_not_blank CHECK (
        display_name IS NULL OR btrim(display_name) <> ''
    )
);

CREATE TABLE public.signal_events (
    cursor BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES public.tenants(id) ON DELETE CASCADE,
    event_id UUID NOT NULL,
    agent_id TEXT NOT NULL,
    sequence NUMERIC(20, 0) NOT NULL,
    received_at_unix_nano NUMERIC(20, 0) NOT NULL,
    signal_kind TEXT NOT NULL,
    payload JSONB NOT NULL,
    payload_bytes BIGINT NOT NULL,
    committed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT signal_events_agent_fk
        FOREIGN KEY (tenant_id, agent_id)
        REFERENCES public.agents(tenant_id, agent_id)
        ON DELETE RESTRICT,
    CONSTRAINT signal_events_tenant_event_unique UNIQUE (tenant_id, event_id),
    CONSTRAINT signal_events_tenant_agent_sequence_unique
        UNIQUE (tenant_id, agent_id, sequence),
    CONSTRAINT signal_events_cursor_positive CHECK (cursor > 0),
    CONSTRAINT signal_events_sequence_non_negative CHECK (sequence >= 0),
    CONSTRAINT signal_events_sequence_u64 CHECK (sequence <= 18446744073709551615),
    CONSTRAINT signal_events_received_at_non_negative CHECK (received_at_unix_nano >= 0),
    CONSTRAINT signal_events_received_at_u64 CHECK (
        received_at_unix_nano <= 18446744073709551615
    ),
    CONSTRAINT signal_events_kind CHECK (signal_kind IN ('metrics', 'traces', 'logs')),
    CONSTRAINT signal_events_payload_object CHECK (jsonb_typeof(payload) = 'object'),
    CONSTRAINT signal_events_payload_bytes_positive CHECK (payload_bytes > 0),
    CONSTRAINT signal_events_payload_bytes_bounded CHECK (payload_bytes <= 16777216)
);

CREATE INDEX signal_events_tenant_cursor_idx
    ON public.signal_events (tenant_id, cursor);
CREATE INDEX signal_events_tenant_committed_at_idx
    ON public.signal_events (tenant_id, committed_at DESC);

CREATE TABLE public.signal_projection (
    tenant_id UUID PRIMARY KEY REFERENCES public.tenants(id) ON DELETE CASCADE,
    last_cursor BIGINT NOT NULL DEFAULT 0,
    projection JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT signal_projection_last_cursor_non_negative CHECK (last_cursor >= 0),
    CONSTRAINT signal_projection_object CHECK (jsonb_typeof(projection) = 'object')
);

CREATE TABLE public.audit_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES public.tenants(id) ON DELETE CASCADE,
    actor_type TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT audit_events_actor_type_not_blank CHECK (btrim(actor_type) <> ''),
    CONSTRAINT audit_events_actor_id_not_blank CHECK (btrim(actor_id) <> ''),
    CONSTRAINT audit_events_action_not_blank CHECK (btrim(action) <> ''),
    CONSTRAINT audit_events_target_type_not_blank CHECK (btrim(target_type) <> ''),
    CONSTRAINT audit_events_target_id_not_blank CHECK (btrim(target_id) <> ''),
    CONSTRAINT audit_events_details_object CHECK (jsonb_typeof(details) = 'object')
);

CREATE INDEX audit_events_tenant_occurred_at_idx
    ON public.audit_events (tenant_id, occurred_at DESC, id DESC);

CREATE TABLE public.stream_tickets (
    ticket_id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES public.tenants(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    authorized_until TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    consumed_at TIMESTAMPTZ,
    CONSTRAINT stream_tickets_subject_not_blank CHECK (btrim(subject) <> ''),
    CONSTRAINT stream_tickets_subject_size CHECK (octet_length(subject) <= 1024),
    CONSTRAINT stream_tickets_authorization_after_creation CHECK (
        authorized_until > created_at
    ),
    CONSTRAINT stream_tickets_expiry_after_creation CHECK (expires_at > created_at),
    CONSTRAINT stream_tickets_expiry_within_authorization CHECK (
        expires_at <= authorized_until
    ),
    CONSTRAINT stream_tickets_consumed_before_expiry CHECK (
        consumed_at IS NULL OR consumed_at < expires_at
    )
);

CREATE INDEX stream_tickets_cleanup_idx
    ON public.stream_tickets (tenant_id, expires_at, consumed_at);

ALTER TABLE public.tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.tenants FORCE ROW LEVEL SECURITY;
CREATE POLICY tenants_tenant_isolation ON public.tenants
    USING (id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE public.agents ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.agents FORCE ROW LEVEL SECURITY;
CREATE POLICY agents_tenant_isolation ON public.agents
    USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE public.signal_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.signal_events FORCE ROW LEVEL SECURITY;
CREATE POLICY signal_events_tenant_isolation ON public.signal_events
    USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE public.signal_projection ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.signal_projection FORCE ROW LEVEL SECURITY;
CREATE POLICY signal_projection_tenant_isolation ON public.signal_projection
    USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE public.audit_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.audit_events FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_events_tenant_isolation ON public.audit_events
    USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);

ALTER TABLE public.stream_tickets ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.stream_tickets FORCE ROW LEVEL SECURITY;
CREATE POLICY stream_tickets_tenant_isolation ON public.stream_tickets
    USING (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid)
    WITH CHECK (tenant_id = NULLIF(current_setting('app.tenant_id', true), '')::uuid);
