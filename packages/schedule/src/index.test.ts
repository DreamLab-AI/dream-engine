import { describe, it, expect } from 'vitest';
import { buildRoutine, serializeRoutine, routineName, scheduleInstructions } from './index.js';
import { defaultConfig } from '@dream-machine/compile';

const cfg = defaultConfig('ruvnet/dream-machine');

describe('routineName', () => {
  it('titles the repo name', () => {
    expect(routineName('ruvnet/metaharness')).toBe('Metaharness Nightly Dream Cycle');
    expect(routineName('acme/my-cool-repo')).toBe('My Cool Repo Nightly Dream Cycle');
  });
});

describe('buildRoutine', () => {
  const r = buildRoutine(cfg, { environmentId: 'env_123', eventUuid: 'abc-uuid' });

  it('produces the /schedule body shape', () => {
    expect(r.cron_expression).toBe('0 8 * * *');
    expect(r.enabled).toBe(true);
    expect(r.job_config.ccr.environment_id).toBe('env_123');
    expect(r.job_config.ccr.session_context.sources[0].git_repository.url).toBe(
      'https://github.com/ruvnet/dream-machine',
    );
    expect(r.job_config.ccr.events[0].data.uuid).toBe('abc-uuid');
  });

  it('embeds the compiled prompt as the user message', () => {
    const content = r.job_config.ccr.events[0].data.message.content;
    expect(content).toContain('Dream Machine — nightly routine for `ruvnet/dream-machine`');
    expect(content).toContain('evaluation is not promotion');
  });

  it('defaults model and tools', () => {
    expect(r.job_config.ccr.session_context.model).toBe('claude-sonnet-5');
    expect(r.job_config.ccr.session_context.allowed_tools).toContain('Bash');
  });

  it('uses a placeholder env + uuid when not supplied', () => {
    const r2 = buildRoutine(cfg);
    expect(r2.job_config.ccr.environment_id).toBe('<ENVIRONMENT_ID>');
    expect(r2.job_config.ccr.events[0].data.uuid).toMatch(/-4000-/);
  });
});

describe('serializeRoutine', () => {
  it('is valid JSON round-trips', () => {
    const json = serializeRoutine(cfg, { environmentId: 'env_x' });
    const parsed = JSON.parse(json);
    expect(parsed.name).toBe('Dream Machine Nightly Dream Cycle');
  });
});

describe('scheduleInstructions', () => {
  it('mentions draft-only and the cron', () => {
    const t = scheduleInstructions(cfg);
    expect(t).toContain('0 8 * * *');
    expect(t).toContain('never merges');
  });
});
