import { useEffect, useRef } from 'react';
import './KeyboardShortcutsModal.css';

function KeyboardShortcutsModal({ isOpen, onClose }) {
  const modalRef = useRef(null);
  const closeButtonRef = useRef(null);

  useEffect(() => {
    if (isOpen) {
      // Auto-focus the close button for accessibility when modal opens
      closeButtonRef.current?.focus();

      const handleEscape = (e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          onClose();
        }
      };

      window.addEventListener('keydown', handleEscape);
      return () => window.removeEventListener('keydown', handleEscape);
    }
  }, [isOpen, onClose]);

  if (!isOpen) {
    return null;
  }

  const shortcuts = [
    {
      description: 'Navigate to Registration (Primary CTA)',
      keys: ['Alt', 'R'],
    },
    {
      description: 'Navigate to Transaction History',
      keys: ['Alt', 'H'],
    },
    {
      description: 'Navigate to Analytics',
      keys: ['Alt', 'A'],
    },
    {
      description: 'Open Keyboard Shortcuts Help',
      keys: ['?'],
    },
    {
      description: 'Close Modal',
      keys: ['Esc'],
    },
  ];

  return (
    <div
      className="shortcuts-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) {
          onClose();
        }
      }}
      data-testid="shortcuts-modal-overlay"
    >
      <div
        className="shortcuts-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-title"
        ref={modalRef}
      >
        <div className="shortcuts-header">
          <h2 id="shortcuts-title" className="shortcuts-title">
            Keyboard Shortcuts
          </h2>
          <button
            type="button"
            className="shortcuts-close-btn"
            onClick={onClose}
            aria-label="Close keyboard shortcuts modal"
            ref={closeButtonRef}
          >
            ✕
          </button>
        </div>
        <div className="shortcuts-body">
          <div className="shortcuts-group">
            {shortcuts.map((sc) => (
              <div key={sc.description} className="shortcuts-row">
                <span className="shortcuts-desc">{sc.description}</span>
                <span className="shortcuts-keys">
                  {sc.keys.map((k, idx) => (
                    <span key={k} style={{ display: 'inline-flex', alignItems: 'center' }}>
                      {idx > 0 && <span style={{ margin: '0 4px', color: '#94a3b8' }}>+</span>}
                      <kbd className="shortcuts-key">{k}</kbd>
                    </span>
                  ))}
                </span>
              </div>
            ))}
          </div>
        </div>
        <div className="shortcuts-footer">
          Press <kbd className="shortcuts-key" style={{ padding: '0 4px', height: '20px', minWidth: '20px' }}>Esc</kbd> anytime to dismiss this modal.
        </div>
      </div>
    </div>
  );
}

export default KeyboardShortcutsModal;
