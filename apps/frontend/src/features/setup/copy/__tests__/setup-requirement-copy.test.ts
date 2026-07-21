import { describe, expect, it } from 'vitest'

import { getSetupRequirementCopy, SETUP_REQUIREMENT_COPY } from '../setup-requirement-copy'

describe('getSetupRequirementCopy', () => {
  it('returns the registered copy for a known id', () => {
    const copy = getSetupRequirementCopy('github_token')
    expect(copy).toBe(SETUP_REQUIREMENT_COPY.github_token)
    expect(copy.userAction?.settingsSection).toBe('github')
    expect(copy.adminAction?.to).toBe('/admin/integrations/github')
  })

  describe('forward compatibility with a newer backend', () => {
    it('falls back to generic copy naming the unrecognised id', () => {
      const copy = getSetupRequirementCopy('some_future_requirement')
      expect(copy.title).toBeTruthy()
      expect(copy.description).toContain('some_future_requirement')
    })

    it('offers no CTA, since we do not know how to fix it', () => {
      const copy = getSetupRequirementCopy('some_future_requirement')
      expect(copy.userAction).toBeUndefined()
      expect(copy.adminAction).toBeUndefined()
    })
  })
})
