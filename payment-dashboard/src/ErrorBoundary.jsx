import { Component } from 'react'

class ErrorBoundary extends Component {
  constructor(props) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error) {
    return { hasError: true, error }
  }

  componentDidCatch(error, errorInfo) {
    console.error('Unhandled error caught by ErrorBoundary:', error, errorInfo)
  }

  handleReload = () => {
    window.location.reload()
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="error-boundary" role="alert">
          <h1>Something went wrong</h1>
          <p>
            An unexpected error occurred while rendering this page. You can try
            reloading the page to recover.
          </p>
          {this.state.error && this.state.error.message ? (
            <pre className="error-boundary-details">
              {this.state.error.message}
            </pre>
          ) : null}
          <button type="button" onClick={this.handleReload}>
            Reload page
          </button>
        </div>
      )
    }

    return this.props.children
  }
}

export default ErrorBoundary
