# Auth0 Groups Setup Guide

This guide explains how to configure Auth0 to include group information in ID tokens, enabling group-based access control in the Moatless application.

## Overview

When using Auth0 for authentication with `STORAGE_TYPE=group`, you can assign users to groups directly within Auth0. The groups are included in the ID token as a custom claim and extracted by the application to determine which group's data the user can access.

## Prerequisites

- Auth0 account with an application configured
- Auth0 credentials set in `.env` file:
  - `AUTH_PROVIDER=auth0`
  - `AUTH0_DOMAIN`
  - `AUTH0_CLIENT_ID`
  - `AUTH0_CLIENT_SECRET`
  - `AUTH0_CALLBACK_URL`

## Step 1: Configure Group Storage

In your `.env` file, set:

```bash
# Enable group-based storage
STORAGE_TYPE=group

# Set groups claim namespace (optional, defaults to https://moatless.ai/groups)
AUTH0_GROUPS_CLAIM=https://moatless.ai/groups
```

## Step 2: Create Auth0 Post-Login Action

1. Go to your Auth0 Dashboard
2. Navigate to **Actions** → **Library**
3. Click **+ Create Action** → **Build from scratch**
4. Name: "Add Groups to ID Token"
5. Trigger: **Login / Post Login**

### Action Code

```javascript
/**
 * Post-Login Action: Add Groups to ID Token
 *
 * This action adds group information to the ID token so the application
 * can determine which group's data the user should access.
 */
exports.onExecutePostLogin = async (event, api) => {
  const namespace = 'https://moatless.ai';

  // Option 1: Get groups from user's app_metadata
  // Groups should be stored as an array in user.app_metadata.groups
  const groups = event.user.app_metadata?.groups || [];

  // Option 2: Use Auth0 Roles (if using Role-Based Access Control)
  // Uncomment this line and comment out the line above to use roles instead
  // const groups = event.authorization?.roles || [];

  if (event.authorization) {
    // Add groups to the ID token with a namespaced claim
    api.idToken.setCustomClaim(`${namespace}/groups`, groups);
  }
};
```

### Important Notes

- **Namespace Required**: Auth0 requires custom claims to use a namespaced URL format to avoid collisions
- **Must Use Your Domain**: Use a URL you control (e.g., `https://moatless.ai`, `https://your-company.com`)
- **Cannot Use Reserved Names**: You cannot use `groups` or `roles` as the claim name without a namespace

## Step 3: Deploy the Action

1. Click **Deploy** in the Action editor
2. Go to **Actions** → **Flows** → **Login**
3. Drag your "Add Groups to ID Token" action from the right panel
4. Place it between **Start** and **Complete**
5. Click **Apply**

## Step 4: Assign Groups to Users

There are two methods to assign groups to users:

### Method A: Using app_metadata (Recommended)

1. Go to **User Management** → **Users**
2. Select a user
3. Scroll to **app_metadata** section
4. Add groups as JSON:

```json
{
  "groups": ["team-frontend", "team-backend"]
}
```

5. Click **Save**

### Method B: Using Auth0 Roles

1. Go to **User Management** → **Roles**
2. Create roles that match your group names (e.g., "team-frontend", "team-backend")
3. Assign roles to users
4. Update your Action code to use `event.authorization.roles` instead of `event.user.app_metadata.groups`

## Step 5: Test the Integration

1. **Start the application** with Auth0 configured
2. **Login** through Auth0
3. **Check application logs** for confirmation:
   ```
   Auth0 user assigned to group: team-frontend
   Created Auth0 session for user john.doe (group: team-frontend)
   ```
4. **Verify group isolation**: Users in different groups should only see their group's data

## Troubleshooting

### User gets "Access denied: You are not assigned to any group"

**Cause**: The user doesn't have groups configured in Auth0

**Solutions**:
1. Add groups to user's `app_metadata` as shown in Step 4
2. Verify the groups claim namespace in your Action matches `AUTH0_GROUPS_CLAIM`
3. Check that the Action is deployed and active in the Login flow

### Groups claim is empty in the ID token

**Possible causes**:
1. **Action not deployed**: Ensure the Action is deployed and added to the Login flow
2. **Wrong namespace**: Verify the namespace in your Action matches `AUTH0_GROUPS_CLAIM` environment variable
3. **Metadata format error**: Ensure `app_metadata.groups` is an array, not a string

### User has multiple groups, wrong one is selected

**Current behavior**: The application uses the **first group** in the array as the primary group

**Solutions**:
1. **Reorder groups**: Put the primary group first in the array
2. **Single group**: Assign users to only one group
3. **Custom implementation**: Modify `Auth0Service.getPrimaryGroup()` to implement your own group selection logic

## Group Configuration Examples

### Single Group per User
```json
{
  "groups": ["team-frontend"]
}
```

### Multiple Groups (First is Primary)
```json
{
  "groups": ["team-frontend", "team-backend", "team-devops"]
}
```
The application will use `team-frontend` as the primary group.

### Using Different Group Formats

The application supports both array and string formats:

**Array format (recommended)**:
```json
{
  "groups": ["team-frontend"]
}
```

**Single string**:
```json
{
  "groups": "team-frontend"
}
```

## Advanced: Custom Groups Claim Namespace

If you need to use a different namespace (e.g., for compliance or existing infrastructure):

1. **Update your Action** to use your namespace:
   ```javascript
   const namespace = 'https://your-company.com';
   api.idToken.setCustomClaim(`${namespace}/groups`, groups);
   ```

2. **Update your `.env` file**:
   ```bash
   AUTH0_GROUPS_CLAIM=https://your-company.com/groups
   ```

## Security Considerations

1. **Groups in ID Token**: Groups are included in the ID token, which is sent to the client. Don't include sensitive information in group names.
2. **Token Validation**: The application validates the ID token signature, ensuring groups cannot be tampered with.
3. **Session Isolation**: Users can only access data from their assigned group, enforced at the storage level.

## Differences from GitHub OAuth

| Feature | GitHub OAuth | Auth0 |
|---------|--------------|-------|
| Group source | Settings file (`settings.json`) | ID token custom claim |
| Configuration | Manual mapping: userId → groupId | Auth0 app_metadata or roles |
| Updates | Requires server restart | Immediate (next login) |
| Scalability | Manual per-user configuration | Centralized in Auth0 |

## Additional Resources

- [Auth0 Actions Documentation](https://auth0.com/docs/customize/actions)
- [Adding Custom Claims to Tokens](https://auth0.com/docs/secure/tokens/json-web-tokens/create-custom-claims)
- [Auth0 Roles Documentation](https://auth0.com/docs/manage-users/access-control/rbac)
- [Moatless Group Storage Documentation](./group-based-storage.md) *(if exists)*
