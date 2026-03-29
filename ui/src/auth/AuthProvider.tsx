import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { type User, type UserManager } from 'oidc-client-ts'
import { initAuth } from './index'

interface AuthState {
  isLoading: boolean
  isAuthenticated: boolean
  user: User | null
  error: Error | null
  /** Redirect the browser to the IdP login page. */
  login: () => Promise<void>
  /** Sign the user out and redirect to post-logout URI. */
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthState | null>(null)

export function AuthProvider({ children }: { children: ReactNode }) {
  const [isLoading, setIsLoading] = useState(true)
  const [user, setUser] = useState<User | null>(null)
  const [error, setError] = useState<Error | null>(null)
  const managerRef = useRef<UserManager | null>(null)

  useEffect(() => {
    let cancelled = false

    async function bootstrap() {
      try {
        const manager = await initAuth()
        managerRef.current = manager

        // Sync user state whenever oidc-client-ts fires events.
        const sync = async () => {
          const current = await manager.getUser()
          if (!cancelled) setUser(current)
        }

        manager.events.addUserLoaded(sync)
        manager.events.addUserUnloaded(sync)
        manager.events.addSilentRenewError((err) => {
          console.error('Silent renew failed:', err)
        })

        // Load any existing session on mount.
        await sync()
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err : new Error(String(err)))
      } finally {
        if (!cancelled) setIsLoading(false)
      }
    }

    bootstrap()
    return () => { cancelled = true }
  }, [])

  const login = useCallback(async () => {
    await managerRef.current?.signinRedirect()
  }, [])

  const logout = useCallback(async () => {
    await managerRef.current?.signoutRedirect()
  }, [])

  return (
    <AuthContext.Provider
      value={{
        isLoading,
        isAuthenticated: user !== null && !user.expired,
        user,
        error,
        login,
        logout,
      }}
    >
      {children}
    </AuthContext.Provider>
  )
}

/** Access the current authentication state. Must be used inside <AuthProvider>. */
export function useAuth(): AuthState {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used within <AuthProvider>')
  return ctx
}
