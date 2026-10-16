export default function LoadingSpinner({
  size = 'w-5 h-5',
  color = 'text-white',
  label = 'Loading...',
  className = '',
  ...props
}) {
  return (
    <span
      role="status"
      aria-label={label}
      className={`inline-flex items-center justify-center ${className}`.trim()}
      {...props}
    >
      <svg
        className={`animate-spin ${size} ${color}`}
        xmlns="http://www.w3.org/2000/svg"
        fill="none"
        viewBox="0 0 24 24"
        aria-hidden="true"
      >
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8z" />
      </svg>
      <span className="visually-hidden sr-only">{label}</span>
    </span>
  )
}
