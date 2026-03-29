import { UserManager, type UserManagerSettings } from 'oidc-client-ts'

export interface SmartConfiguration {
  authorization_endpoint: string
  token_endpoint: string
  jwks_uri?: string
  scopes_supported?: string[]
  token_endpoint_auth_methods_supported?: string[]
}

let _userManager: UserManager | null = null

/**
 * Fetch SMART configuration from HFS and initialize the UserManager.
 * Must be called once before any auth operations.
 *
 * The HFS base URL is read from VITE_HFS_BASE_URL (defaults to /api for
 * the Vite dev proxy, or the same origin in production).
 */
export async function initAuth(): Promise<UserManager> {
  if (_userManager) return _userManager

  const hfsBase = import.meta.env.VITE_HFS_BASE_URL ?? '/api'
  const clientId = import.meta.env.VITE_OIDC_CLIENT_ID ?? 'hfs-ui'
  const scopes = import.meta.env.VITE_OIDC_SCOPES ?? 'openid profile user/*.rs'

  const discoveryUrl = `${hfsBase}/.well-known/smart-configuration`
  const response = await fetch(discoveryUrl)
  if (!response.ok) {
    throw new Error(
      `Failed to fetch SMART configuration from ${discoveryUrl}: ${response.status} ${response.statusText}`
    )
  }
  const smart: SmartConfiguration = await response.json()

  if (!smart.authorization_endpoint || !smart.token_endpoint) {
    throw new Error(
      'SMART configuration is incomplete: missing authorization_endpoint or token_endpoint. ' +
      'Set HFS_SMART_AUTHORIZE_ENDPOINT and HFS_SMART_TOKEN_ENDPOINT on the HFS server.'
    )
  }

  const settings: UserManagerSettings = {
    // Use the issuer URL as authority so oidc-client-ts knows where to validate tokens.
    // We supply explicit metadata to avoid a second discovery fetch.
    authority: new URL(smart.authorization_endpoint).origin,
    metadata: {
      authorization_endpoint: smart.authorization_endpoint,
      token_endpoint: smart.token_endpoint,
      jwks_uri: smart.jwks_uri,
      end_session_endpoint: undefined,
    },
    client_id: clientId,
    redirect_uri: `${window.location.origin}/callback`,
    post_logout_redirect_uri: window.location.origin,
    scope: scopes,
    response_type: 'code',
    // oidc-client-ts uses PKCE (S256) by default for code flow — no extra config needed.
    // Access tokens are stored in memory via the default sessionStorage stateStore,
    // but the User (with access_token) is kept in memory only — not localStorage.
    userStore: undefined, // defaults to SessionStorage for state; access token in memory
    automaticSilentRenew: true,
    // Renew 60 seconds before expiry to ensure uninterrupted API access.
    accessTokenExpiringNotificationTimeInSeconds: 60,
  }

  _userManager = new UserManager(settings)
  return _userManager
}

/** Returns the initialized UserManager. Throws if initAuth() has not been called. */
export function getUserManager(): UserManager {
  if (!_userManager) {
    throw new Error('Auth not initialized. Call initAuth() first.')
  }
  return _userManager
}
