/**
 * Reusable prompts for Claude Code agent interactions
 */

/**
 * Prompt template for generating branch names and summaries from user requests.
 * This prompt provides detailed guidelines for creating good branch names and
 * expects a JSON response with branchName and summary.
 *
 * @param request - The user request to analyze
 * @returns Formatted prompt string with the request embedded
 */
export function createBranchNamePrompt(request: string): string {
  return `You are tasked with creating a branch name and a summary for a given request. This information will be used in a version control system for tracking changes and organizing work. Your response must be in a strict JSON format.

Guidelines for creating a good branch name:
- Use lowercase letters
- Replace spaces with hyphens
- Keep it concise but descriptive (ideally 2-4 words)
- Avoid special characters other than hyphens
- Start with a verb or category (e.g., "feature", "bugfix", "refactor")

The request you need to analyze is provided below:

<request>
${request}
</request>

Analyze the request carefully. Based on the content, create a suitable branch name that follows the guidelines mentioned above. The branch name should reflect the main purpose or feature of the request.

Next, create a brief summary of the task described in the request. This summary should be concise (1 sentence, max 10 words) but capture the essential elements of the task.

Your final output must be in the following JSON format:

<output>
{
  "branchName": "your-branch-name-here",
  "summary": "Your concise summary of the task here."
}
</output>

Ensure that your response contains only the JSON object within the <output> tags. Do not include any additional text, explanations, or formatting outside of the JSON structure.`
}

/**
 * Common system prompts for Claude Code agent configurations
 */
export const CLAUDE_CODE_SYSTEM_PROMPTS = {
  /**
   * System prompt for branch name generation tasks
   */
  BRANCH_NAME_GENERATION:
    'You are an expert at analyzing software development requests and creating appropriate branch names and summaries. Focus on being concise, following git conventions, and capturing the essence of the request.',

  /**
   * System prompt for general task assistance
   */
  GENERAL_ASSISTANCE:
    'You are a helpful assistant that can analyze code, help with development tasks, and provide technical guidance. Always be precise and follow best practices.',
} as const
