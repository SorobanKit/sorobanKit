import { describe, it, expect, vi, afterEach } from 'vitest'
import { render, screen } from '@testing-library/react'
import ErrorBoundary from './ErrorBoundary.jsx'

function Bomb() {
  throw new Error('boom')
}

describe('ErrorBoundary', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders the fallback UI when a child component throws', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})

    render(
      <ErrorBoundary>
        <Bomb />
      </ErrorBoundary>,
    )

    expect(screen.getByRole('alert')).toBeDefined()
    expect(screen.getByText(/something went wrong/i)).toBeDefined()
    expect(screen.getByRole('button', { name: /reload page/i })).toBeDefined()
  })

  it('renders children when no error is thrown', () => {
    render(
      <ErrorBoundary>
        <div>All good</div>
      </ErrorBoundary>,
    )

    expect(screen.getByText('All good')).toBeDefined()
  })
})
