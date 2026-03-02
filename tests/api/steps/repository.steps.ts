/**
 * Repository step functions (REPO-01 through REPO-07).
 *
 * Each function performs an action and returns a result.
 * Tests add assertions; preconditions skip them.
 */

import type {
	ApiClient,
	CreateRepositoryRequest,
	RepositoryResponse,
	UpdateRepositoryRequest,
} from "../../shared/api-client";

// =============================================================================
// REPO-01: Create repository from GitHub URL
// =============================================================================

export async function createRepository(
	api: ApiClient,
	config: {
		name: string;
		remoteUrl: string;
		kind?: "remote" | "template" | "git";
		provider?: "github" | "gitlab" | "gitness";
		description?: string;
	},
): Promise<{ repositoryId: string; repository: RepositoryResponse }> {
	const repository = await api.createRepository({
		name: config.name,
		kind: config.kind ?? "remote",
		remoteUrl: config.remoteUrl,
		provider: config.provider ?? "github",
		description: config.description,
	});
	return { repositoryId: repository.id, repository };
}

// =============================================================================
// REPO-02: Get repository by ID
// =============================================================================

export async function getRepository(
	api: ApiClient,
	repositoryId: string,
): Promise<RepositoryResponse> {
	return api.getRepository(repositoryId);
}

// =============================================================================
// REPO-03: Update repository metadata
// =============================================================================

export async function updateRepository(
	api: ApiClient,
	repositoryId: string,
	data: UpdateRepositoryRequest,
): Promise<RepositoryResponse> {
	return api.updateRepository(repositoryId, data);
}

// =============================================================================
// REPO-04: List repositories
// =============================================================================

export async function listRepositories(
	api: ApiClient,
): Promise<RepositoryResponse[]> {
	return api.listRepositories();
}

// =============================================================================
// REPO-05: Delete repository
// =============================================================================

export async function deleteRepository(
	api: ApiClient,
	repositoryId: string,
): Promise<Response> {
	return api.deleteRepositoryRaw(repositoryId);
}

// =============================================================================
// REPO-06: Create repository with missing required fields (error case)
// =============================================================================

export async function createRepositoryWithMissingFields(
	api: ApiClient,
): Promise<Response> {
	// Send empty body — backend requires at least `name`
	return api.createRepositoryRaw({} as CreateRepositoryRequest);
}

// =============================================================================
// REPO-07: Get non-existent repository (error case)
// =============================================================================

export async function getNonExistentRepository(
	api: ApiClient,
	fakeId: string,
): Promise<Response> {
	return api.getRepositoryRaw(fakeId);
}
