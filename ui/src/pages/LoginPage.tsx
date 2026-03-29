import { useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { useAuth } from '../auth/AuthProvider'

export function LoginPage() {
  const { isLoading, isAuthenticated, error, login } = useAuth()
  const navigate = useNavigate()

  // If already authenticated, redirect to the dashboard.
  useEffect(() => {
    if (!isLoading && isAuthenticated) {
      navigate('/', { replace: true })
    }
  }, [isLoading, isAuthenticated, navigate])

  if (isLoading) {
    return <p>Loading...</p>
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', paddingTop: '8rem', gap: '1.5rem' }}>
      <h1>HFS FHIR Server</h1>
      <p style={{ color: '#666', fontSize: '1rem' }}>
        Sign in with your organization account to access FHIR resources.
      </p>

      {error && (
        <p style={{ color: '#c00', fontSize: '0.875rem' }}>
          Authentication error: {error.message}
        </p>
      )}

      <button
        onClick={login}
        style={{ padding: '0.75rem 2rem', fontSize: '1rem', cursor: 'pointer' }}
      >
        Sign in
      </button>

      <p style={{ fontSize: '0.75rem', color: '#999', maxWidth: '28rem', textAlign: 'center' }}>
        You will be redirected to your identity provider. Authentication uses PKCE — no passwords
        are sent to this application.
      </p>
    </div>
  )
}
