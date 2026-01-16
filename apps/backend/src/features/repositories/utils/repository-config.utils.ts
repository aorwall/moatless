import type { RepositoryConfig, RepositoryConfigFile } from '@moatless/types'
import { schemas } from '@moatless/types'
import type { GitProviderClient } from '../../../infrastructure/clients/git-provider.interface'
import { logger } from '../../../utils/logger'

const CONFIG_FILE_PATH = '.moatless/config.json'

/**
 * Read repository configuration from remote repository
 * Fetches .moatless/config.json from the repository using the provider API
 */
export async function readRepoConfigFile(
  remoteUrl: string,
  provider: GitProviderClient,
  token?: string,
  ref?: string,
): Promise<RepositoryConfigFile | null> {
  try {
    const content = await provider.getFileContent(remoteUrl, CONFIG_FILE_PATH, token, ref)

    if (!content) {
      logger.debug({ remoteUrl, path: CONFIG_FILE_PATH }, 'Config file not found in repository')
      return null
    }

    const parsed = JSON.parse(content)
    const result = schemas.RepositoryConfigFileSchema.safeParse(parsed)

    if (!result.success) {
      logger.warn(
        { remoteUrl, path: CONFIG_FILE_PATH, issues: result.error.issues },
        'Invalid config file format',
      )
      return null
    }

    logger.info(
      { remoteUrl, path: CONFIG_FILE_PATH },
      'Successfully read config file from repository',
    )
    return result.data
  } catch (error) {
    logger.error(
      { err: error, remoteUrl, path: CONFIG_FILE_PATH },
      'Error reading config file from repository',
    )
    return null
  }
}

/**
 * Merge config file data into repository configuration
 * Config file values are used as defaults - they don't override existing values
 */
export function mergeConfigIntoRepository<T extends RepositoryConfig>(
  repository: T,
  configFile: RepositoryConfigFile,
): T {
  return {
    ...repository,
    // Only use config file values if repository doesn't have them set
    systemPrompt: repository.systemPrompt ?? configFile.systemPrompt,
    codeExecution: mergeCodeExecution(repository.codeExecution, configFile.codeExecution),
    previewConfig: repository.previewConfig ?? configFile.previewConfig,
  }
}

/**
 * Merge code execution config, preferring existing values
 */
function mergeCodeExecution(
  existing: RepositoryConfig['codeExecution'],
  fromFile: RepositoryConfigFile['codeExecution'],
): RepositoryConfig['codeExecution'] {
  if (!fromFile) {
    return existing
  }

  if (!existing || Object.keys(existing).length === 0) {
    return fromFile
  }

  // Merge field by field, preferring existing values
  return {
    dockerImage: existing.dockerImage ?? fromFile.dockerImage,
    setupCommands: existing.setupCommands ?? fromFile.setupCommands,
    envVars: existing.envVars ?? fromFile.envVars,
  }
}
