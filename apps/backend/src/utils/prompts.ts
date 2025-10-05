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

Here are two examples:

Example 1:
Request: "Add user authentication with login and logout functionality"
Response: {"branchName": "feature/user-authentication", "summary": "Add user login and logout functionality"}

Example 2:
Request: "Fix the memory leak in the task event streaming component"
Response: {"branchName": "fix/memory-leak-streaming", "summary": "Fix memory leak in task streaming"}

The request you need to analyze is provided below:

<request>
${request}
</request>

Analyze the request carefully. Based on the content, create a suitable branch name that follows the guidelines mentioned above. The branch name should reflect the main purpose or feature of the request.

Next, create a brief summary of the task described in the request. This summary should be concise (1 sentence, max 10 words) but capture the essential elements of the task.

Your response must be in the following JSON format:

{
  "branchName": "your-branch-name-here",
  "summary": "Your concise summary of the task here."
}

Respond with only the JSON object. Do not include any additional text, explanations, or formatting.`
}

export function createRepositoryPrompt(request: string): string {
  return `You are tasked with creating a repository name for a web application based on a given request. This repository name will be used to host the web app's source code and should clearly represent the application's purpose and functionality. Your response must be in a strict JSON format.
Guidelines for creating a good repository name:

Use lowercase letters
Replace spaces with hyphens
Keep it concise but descriptive (ideally 2-4 words)
Avoid special characters other than hyphens and underscores
Should reflect the main purpose or domain of the web app
Can include common suffixes like "-app", "-web", "-platform", "-dashboard"

Here are two examples:
Example 1:
Request: "Create a web application for managing personal finances and budget tracking"
Response: {"repositoryName": "budget-tracker-app", "description": "Web application for personal finance and budget management"}
Example 2:
Request: "Build a platform for online code collaboration and real-time editing"
Response: {"repositoryName": "code-collab-platform", "description": "Real-time code collaboration and editing platform"}
The request you need to analyze is provided below:
<request>
${request}
</request>
Analyze the request carefully. Based on the content, create a suitable repository name that follows the guidelines mentioned above. The repository name should reflect the main purpose and functionality of the web application described in the request.
Next, create a brief description of the web application described in the request. This description should be concise (1-2 sentences, max 15 words) but capture the essential purpose and key features of the application.
Your response must be in the following JSON format:
{
"repositoryName": "your-repository-name-here",
"description": "Your concise description of the web application here."
}
Respond with only the JSON object. Do not include any additional text, explanations, or <formatting class=""></formatting>`
}
