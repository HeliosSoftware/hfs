import { type ReactNode } from 'react'
import { Navigate } from 'react-router-dom'
import { useAuth } from '../auth/AuthProvider'

interface Props {
  children: ReactNode
}

/**
 * Wraps a route that requires authentication.
 * Redirects unauthenticated users to /login.
 * Shows a loading indicator while the session is being restored.
 */
export function ProtectedRoute({ children }: Props) {
  const { isLoading, isAuthenticated } = useAuth()

  if (isLoading) {
    return (
      <div style={{ display: 'flex', justifyContent: 'center', padding: '4rem' }}>
        <p>Loading...</p>
      </div>
    )
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" replace />
  }

  return <>{children}</>
}
