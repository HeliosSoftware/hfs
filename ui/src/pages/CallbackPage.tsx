import { useEffect, useRef } from 'react'
import { useNavigate } from 'react-router-dom'
import { initAuth } from '../auth'

/**
 * Handles the OAuth2 authorization code callback.
 *
 * The IdP redirects the browser here after the user authenticates.
 * oidc-client-ts completes the PKCE exchange, stores the tokens in the
 * UserManager, and we redirect the user to the dashboard.
 */
export function CallbackPage() {
  const navigate = useNavigate()
  // Use a ref to ensure the callback is processed exactly once even in StrictMode.
  const processed = useRef(false)

  useEffect(() => {
    if (processed.current) return
    processed.current = true

    async function handleCallback() {
      try {
        const manager = await initAuth()
        await manager.signinRedirectCallback()
        navigate('/', { replace: true })
      } catch (err) {
        console.error('Callback error:', err)
        navigate('/login', { replace: true })
      }
    }

    handleCallback()
  }, [navigate])

  return (
    <div style={{ display: 'flex', justifyContent: 'center', padding: '4rem' }}>
      <p>Completing sign-in...</p>
    </div>
  )
}
