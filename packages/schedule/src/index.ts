/**
 * @dream-machine/schedule
 *
 * Turns a compiled routine prompt into the JSON body the cloud `/schedule`
 * routine API expects. v1 is "compile → paste into /schedule"; the same body
 * shape is ready for an automated POST once a callable scheduling API is
 * confirmed. No network calls happen here — this is pure body construction.
 */
import { compile, type DreamConfig, withDefaults } from '@dream-machine/compile';

export interface ScheduleOptions {
  /** Cloud environment id the routine runs in. */
  environmentId?: string;
  /** Model id for the session. Default claude-sonnet-5. */
  model?: string;
  /** Tools the routine session is allowed to use. */
  allowedTools?: string[];
  /** A deterministic uuid for the event (else a fixed placeholder is used). */
  eventUuid?: string;
  /** Human-readable routine name. Default derived from repo. */
  name?: string;
}

export interface RoutineBody {
  name: string;
  cron_expression: string;
  enabled: boolean;
  job_config: {
    ccr: {
      environment_id: string;
      session_context: {
        model: string;
        sources: Array<{ git_repository: { url: string } }>;
        allowed_tools: string[];
      };
      events: Array<{
        data: {
          uuid: string;
          session_id: string;
          type: 'user';
          parent_tool_use_id: null;
          message: { role: 'user'; content: string };
        };
      }>;
    };
  };
}

const DEFAULT_TOOLS = [
  'Bash',
  'Read',
  'Write',
  'Edit',
  'MultiEdit',
  'Glob',
  'Grep',
  'WebFetch',
  'WebSearch',
  'Task',
  'TodoWrite',
  'BashOutput',
  'KillBash',
];

const PLACEHOLDER_UUID = '00000000-0000-4000-8000-000000000000';

/** Derive a routine display name from a repo slug. */
export function routineName(repo: string): string {
  const name = repo.split('/')[1] ?? repo;
  const title = name.replace(/[-_]/g, ' ').replace(/\b\w/g, (m) => m.toUpperCase());
  return `${title} Nightly Dream Cycle`;
}

/** Build the routine body from a config (compiling the prompt internally). */
export function buildRoutine(config: DreamConfig, opts: ScheduleOptions = {}): RoutineBody {
  const c = withDefaults(config);
  const content = compile(config);
  return {
    name: opts.name ?? routineName(config.repo),
    cron_expression: c.cron,
    enabled: true,
    job_config: {
      ccr: {
        environment_id: opts.environmentId ?? '<ENVIRONMENT_ID>',
        session_context: {
          model: opts.model ?? 'claude-sonnet-5',
          sources: [{ git_repository: { url: `https://github.com/${config.repo}` } }],
          allowed_tools: opts.allowedTools ?? DEFAULT_TOOLS,
        },
        events: [
          {
            data: {
              uuid: opts.eventUuid ?? PLACEHOLDER_UUID,
              session_id: '',
              type: 'user',
              parent_tool_use_id: null,
              message: { role: 'user', content },
            },
          },
        ],
      },
    },
  };
}

/** Serialize a routine body to pretty JSON for paste-in. */
export function serializeRoutine(config: DreamConfig, opts: ScheduleOptions = {}): string {
  return JSON.stringify(buildRoutine(config, opts), null, 2);
}

/** Human instructions for the paste-in flow. */
export function scheduleInstructions(config: DreamConfig, opts: ScheduleOptions = {}): string {
  const c = withDefaults(config);
  return [
    `# Schedule the ${routineName(config.repo)} (cron ${c.cron} UTC)`,
    ``,
    `1. Run:  dream-machine schedule --out routine.json`,
    `2. In Claude Code, invoke /schedule and create a routine with the body in routine.json`,
    `   (or use the RemoteTrigger "create" action with this body).`,
    `3. Set environment_id to your cloud environment.`,
    `4. The nightly run writes gists, issues, and DRAFT PRs — it never merges.`,
    opts.environmentId ? `   (environment_id is pre-filled: ${opts.environmentId})` : '',
  ]
    .filter(Boolean)
    .join('\n');
}
