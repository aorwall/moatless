import { describe, expect, it } from 'vitest'
import { updateSettingsSchema } from '../settings.schemas.js'

describe('Settings Schemas', () => {
  describe('updateSettingsSchema', () => {
    it('should validate empty object', () => {
      const result = updateSettingsSchema.safeParse({})
      expect(result.success).toBe(true)
      expect(result.data).toEqual({})
    })

    it('should validate valid dataDir', () => {
      const validInputs = [{ dataDir: '/valid/path' }, { dataDir: null }, { dataDir: '' }]

      for (const input of validInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(true)
        expect(result.data).toEqual(input)
      }
    })

    it('should reject invalid dataDir', () => {
      const invalidInputs = [{ dataDir: 123 }, { dataDir: true }, { dataDir: [] }, { dataDir: {} }]

      for (const input of invalidInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(false)
      }
    })

    it('should validate valid defaultWorkspaceType', () => {
      const validInputs = [
        { defaultWorkspaceType: 'docker' },
        { defaultWorkspaceType: 'local' },
        { defaultWorkspaceType: null },
      ]

      for (const input of validInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(true)
        expect(result.data).toEqual(input)
      }
    })

    it('should reject invalid defaultWorkspaceType', () => {
      const invalidInputs = [
        { defaultWorkspaceType: 'kubernetes' },
        { defaultWorkspaceType: 'remote' },
        { defaultWorkspaceType: '' },
        { defaultWorkspaceType: 123 },
        { defaultWorkspaceType: true },
      ]

      for (const input of invalidInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(false)
      }
    })

    it('should validate valid claude settings', () => {
      const validInputs = [
        {
          claude: {
            binDir: '/valid/bin',
            configDir: '/valid/config',
          },
        },
        {
          claude: {
            binDir: null,
            configDir: null,
          },
        },
        {
          claude: {
            binDir: '',
            configDir: '',
          },
        },
        {
          claude: {
            binDir: '/valid/bin',
          },
        },
        {
          claude: {
            configDir: '/valid/config',
          },
        },
        {
          claude: {},
        },
      ]

      for (const input of validInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(true)
        expect(result.data).toEqual(input)
      }
    })

    it('should reject invalid claude settings', () => {
      const invalidInputs = [
        {
          claude: {
            binDir: 123,
          },
        },
        {
          claude: {
            configDir: true,
          },
        },
        {
          claude: {
            binDir: [],
          },
        },
        {
          claude: {
            configDir: {},
          },
        },
        {
          claude: {
            invalidField: 'value',
          },
        },
      ]

      for (const input of invalidInputs) {
        const result = updateSettingsSchema.safeParse(input)
        expect(result.success).toBe(false)
      }
    })

    it('should validate complete valid settings object', () => {
      const validSettings = {
        dataDir: '/custom/data',
        defaultWorkspaceType: 'docker' as const,
        claude: {
          binDir: '/custom/bin',
          configDir: '/custom/config',
        },
      }

      const result = updateSettingsSchema.safeParse(validSettings)
      expect(result.success).toBe(true)
      expect(result.data).toEqual(validSettings)
    })

    it('should handle null values correctly', () => {
      const settingsWithNulls = {
        dataDir: null,
        defaultWorkspaceType: null,
        claude: {
          binDir: null,
          configDir: null,
        },
      }

      const result = updateSettingsSchema.safeParse(settingsWithNulls)
      expect(result.success).toBe(true)
      expect(result.data).toEqual(settingsWithNulls)
    })

    it('should allow undefined values (optional fields)', () => {
      const settingsWithUndefined = {
        dataDir: undefined,
        defaultWorkspaceType: undefined,
        claude: {
          binDir: undefined,
          configDir: undefined,
        },
      }

      const result = updateSettingsSchema.safeParse(settingsWithUndefined)
      expect(result.success).toBe(true)
      // Undefined values should be preserved in the result
      expect(result.data).toEqual(settingsWithUndefined)
    })

    it('should reject unknown fields at root level', () => {
      const settingsWithUnknownField = {
        dataDir: '/valid/path',
        unknownField: 'value',
      }

      const result = updateSettingsSchema.safeParse(settingsWithUnknownField)
      expect(result.success).toBe(false)
    })

    it('should handle mixed valid and invalid fields', () => {
      const mixedSettings = {
        dataDir: '/valid/path', // valid
        defaultWorkspaceType: 'invalid-type', // invalid
        claude: {
          binDir: '/valid/bin', // valid
          configDir: 123, // invalid
        },
      }

      const result = updateSettingsSchema.safeParse(mixedSettings)
      expect(result.success).toBe(false)
    })

    it('should maintain type safety', () => {
      const validSettings = {
        dataDir: '/path',
        defaultWorkspaceType: 'docker' as const,
        claude: {
          binDir: '/bin',
          configDir: '/config',
        },
      }

      const result = updateSettingsSchema.safeParse(validSettings)

      if (result.success) {
        // Type checking - these should all be properly typed
        const dataDir: string | null | undefined = result.data.dataDir
        const workspaceType: 'docker' | 'local' | null | undefined =
          result.data.defaultWorkspaceType
        const binDir: string | null | undefined = result.data.claude?.binDir
        const configDir: string | null | undefined = result.data.claude?.configDir

        expect(dataDir).toBe('/path')
        expect(workspaceType).toBe('docker')
        expect(binDir).toBe('/bin')
        expect(configDir).toBe('/config')
      }
    })
  })
})
