#!/bin/bash

# Bulk find and replace script for environment -> repository refactoring
echo "Starting bulk environment -> repository refactoring..."

# Define file patterns to search
SEARCH_DIRS="client/src server/services server/routes shared/types shared/schemas"
FILE_PATTERNS="-name '*.ts' -o -name '*.tsx' -o -name '*.js' -o -name '*.jsx'"

# Function to do safe replacements (avoiding process.env, NODE_ENV, etc.)
safe_replace() {
    local from="$1"
    local to="$2"
    local description="$3"
    
    echo "Replacing $description..."
    
    # Find all TypeScript/JavaScript files and replace
    find $SEARCH_DIRS $FILE_PATTERNS -type f -exec grep -l "$from" {} \; 2>/dev/null | while read file; do
        # Skip if it contains NODE_ENV or process.env (system environment variables)
        if ! grep -q "NODE_ENV\|process\.env\|environment.*node\|environment.*jsdom" "$file"; then
            echo "  Updating: $file"
            sed -i.bak "s/$from/$to/g" "$file"
            rm "$file.bak" 2>/dev/null || true
        fi
    done
}

# 1. Replace type names and interfaces
safe_replace "Environment\b" "RepositoryConfig" "Environment type to RepositoryConfig"
safe_replace "EnvironmentResponse" "RepositoryResponse" "EnvironmentResponse type"
safe_replace "EnvironmentList" "RepositoryList" "EnvironmentList type"
safe_replace "CreateEnvironmentRequest" "CreateRepositoryRequest" "Create request type"
safe_replace "UpdateEnvironmentRequest" "UpdateRepositoryRequest" "Update request type"
safe_replace "EnvironmentService" "RepositoryService" "Service class name"

# 2. Replace method names
safe_replace "createEnvironment" "createRepository" "create method"
safe_replace "getEnvironment" "getRepository" "get method"
safe_replace "listEnvironments" "listRepositories" "list method"
safe_replace "updateEnvironment" "updateRepository" "update method"
safe_replace "deleteEnvironment" "deleteRepository" "delete method"
safe_replace "saveEnvironment" "saveRepository" "save method"

# 3. Replace variable names and properties
safe_replace "environmentId" "repositoryId" "ID property"
safe_replace "envId" "repoId" "short ID variable"
safe_replace "environment:" "repository:" "object property"

# 4. Replace API endpoints
safe_replace "/environments/" "/repositories/" "API endpoints"
safe_replace "/environments\\\$" "/repositories" "API endpoints (end)"

# 5. Replace query keys
safe_replace "environments()" "repositories()" "query keys"
safe_replace "environmentsList" "repositoriesList" "query keys"
safe_replace "environmentDetail" "repositoryDetail" "query keys"
safe_replace "environmentFileSearch" "repositoryFileSearch" "query keys"
safe_replace "environmentBranches" "repositoryBranches" "query keys"

# 6. Replace error classes
safe_replace "EnvironmentNotFoundError" "RepositoryNotFoundError" "error classes"

# 7. Replace UI text
safe_replace "Environment" "Repository" "UI text (capitalized)"
safe_replace "environment" "repository" "UI text (lowercase)"
safe_replace "Environments" "Repositories" "UI text (plural)"
safe_replace "environments" "repositories" "UI text (plural lowercase)"

echo "Bulk replacements completed!"
echo "Note: This script avoided files containing NODE_ENV, process.env, or test environment settings"