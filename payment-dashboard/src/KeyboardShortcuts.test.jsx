// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import KeyboardShortcutsModal from './components/KeyboardShortcutsModal';
import App from './App';

// Mock wallet kit and freighter
vi.mock('@creit.tech/stellar-wallets-kit', () => {
  class Stub {
    openModal() {}
    setWallet() {}
    getAddress() {
      return Promise.resolve('GABC123');
    }
  }
  return {
    StellarWalletsKit: Stub,
    WalletNetwork: { TESTNET: 'TESTNET' },
    FreighterModule: Stub,
    xBullModule: Stub,
    AlbedoModule: Stub,
    LobstrModule: Stub,
  };
});

vi.mock('@stellar/freighter-api', () => ({
  default: {
    isConnected: vi.fn().mockResolvedValue(true),
    requestAccess: vi.fn().mockResolvedValue({ address: 'GABC123' }),
    signMessage: vi.fn().mockResolvedValue({ signedMessage: 'sig123' }),
  },
}));

describe('KeyboardShortcutsModal Component', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('renders nothing when isOpen is false', () => {
    const { container } = render(<KeyboardShortcutsModal isOpen={false} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders dialog and lists all required shortcuts when isOpen is true', () => {
    render(<KeyboardShortcutsModal isOpen={true} onClose={() => {}} />);
    
    const dialog = screen.getByRole('dialog');
    expect(dialog).toBeDefined();
    expect(screen.getByText('Keyboard Shortcuts')).toBeDefined();

    // Verify presence of all key descriptions
    expect(screen.getByText(/Navigate to Registration/i)).toBeDefined();
    expect(screen.getByText(/Navigate to Transaction History/i)).toBeDefined();
    expect(screen.getByText(/Navigate to Analytics/i)).toBeDefined();
    expect(screen.getByText(/Open Keyboard Shortcuts Help/i)).toBeDefined();

    // Verify key tags
    const keys = screen.getAllByText('Alt');
    expect(keys.length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText('R')).toBeDefined();
    expect(screen.getByText('H')).toBeDefined();
    expect(screen.getByText('A')).toBeDefined();
    expect(screen.getByText('?')).toBeDefined();
  });

  it('calls onClose when clicking close button', () => {
    const handleClose = vi.fn();
    render(<KeyboardShortcutsModal isOpen={true} onClose={handleClose} />);

    const closeBtn = screen.getByRole('button', { name: /close keyboard shortcuts modal/i });
    fireEvent.click(closeBtn);
    expect(handleClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when pressing Escape key', () => {
    const handleClose = vi.fn();
    render(<KeyboardShortcutsModal isOpen={true} onClose={handleClose} />);

    fireEvent.keyDown(window, { key: 'Escape' });
    expect(handleClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when clicking on the backdrop overlay', () => {
    const handleClose = vi.fn();
    render(<KeyboardShortcutsModal isOpen={true} onClose={handleClose} />);

    const overlay = screen.getByTestId('shortcuts-modal-overlay');
    fireEvent.click(overlay);
    expect(handleClose).toHaveBeenCalledTimes(1);
  });
});

describe('App Keyboard Navigation and Global Shortcuts', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    window.location.hash = '';
  });

  it('toggles shortcuts modal when pressing ? key', async () => {
    render(<App />);

    // Initially modal is closed
    expect(screen.queryByRole('dialog')).toBeNull();

    // Press ?
    fireEvent.keyDown(window, { key: '?' });
    expect(screen.getByRole('dialog')).toBeDefined();
    expect(screen.getByText('Keyboard Shortcuts')).toBeDefined();

    // Press ? again to toggle closed
    fireEvent.keyDown(window, { key: '?' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('closes modal when Escape is pressed while open', async () => {
    render(<App />);

    fireEvent.keyDown(window, { key: '?' });
    expect(screen.getByRole('dialog')).toBeDefined();

    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('does not open modal when typing ? inside an input field', async () => {
    render(<App />);

    // Create an input in the DOM and focus it
    const input = document.createElement('input');
    document.body.appendChild(input);
    input.focus();

    fireEvent.keyDown(input, { key: '?' });
    expect(screen.queryByRole('dialog')).toBeNull();

    document.body.removeChild(input);
  });

  it('navigates to register when Alt+R is pressed', async () => {
    render(<App />);

    fireEvent.keyDown(window, { key: 'r', altKey: true });
    expect(window.location.hash).toBe('#register');
  });

  it('navigates to history when Alt+H is pressed', async () => {
    render(<App />);

    fireEvent.keyDown(window, { key: 'h', altKey: true });
    expect(window.location.hash).toBe('#history');
  });

  it('navigates to analytics when Alt+A is pressed', async () => {
    render(<App />);

    fireEvent.keyDown(window, { key: 'a', altKey: true });
    expect(window.location.hash).toBe('#analytics');
  });

  it('does not trigger navigation when typing Alt+R inside an input field', async () => {
    render(<App />);
    window.location.hash = '';

    const input = document.createElement('input');
    document.body.appendChild(input);
    input.focus();

    fireEvent.keyDown(input, { key: 'r', altKey: true });
    expect(window.location.hash).toBe('');

    document.body.removeChild(input);
  });
});
