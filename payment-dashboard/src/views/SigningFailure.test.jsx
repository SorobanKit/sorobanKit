// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import Dashboard from './Dashboard';
import RegistrationPage from './RegistrationPage';

// Mock dependencies for Dashboard
vi.mock('@stellar/freighter-api', () => ({
  default: {
    signTransaction: vi.fn(),
  },
  signTransaction: vi.fn(),
}));

vi.mock('@stellar-tags/payment-router', () => {
  class MockClient {
    async route_payment() {
      return {
        toXDR: () => 'AAAA_MOCK_XDR_STRING',
      };
    }
  }
  return {
    Client: MockClient,
    networks: {
      testnet: {
        networkPassphrase: 'Test SDF Network ; September 2015',
        contractId: 'CDNQ7OMHIFOLZHOKWQLOGDW7CF3DRMKXJC6OULNGNBWF4O4NO2NEIGER',
      },
    },
  };
});

// Mock walletKit in shared.js
const mockSignMessage = vi.fn();
vi.mock('./shared', () => ({
  CONTRACT_ID: 'CDNQ7OMHIFOLZHOKWQLOGDW7CF3DRMKXJC6OULNGNBWF4O4NO2NEIGER',
  TREASURY_ADDRESS: 'GAAFWEZKDYPXLTQGKQ3F23TXWYQUDAYTDW7P7VUQSVJFW2GWC4Y6LWST',
  TOKEN_ADDRESS: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
  API_BASE: 'https://test-api.example.com',
  DEFAULT_FEDERATION_DOMAIN: 'localhost',
  HORIZON_BASE: 'https://horizon-testnet.stellar.org',
  walletKit: {
    signMessage: (...args) => mockSignMessage(...args),
  },
  useNavState: () => [false, vi.fn()],
  useWalletMenu: () => ({ menuRef: { current: null }, isOpen: false, setIsOpen: vi.fn() }),
  formatShortAddress: (val) => val ? `${val.substring(0, 4)}...${val.substring(val.length - 4)}` : '',
  formatUsername: (val) => val ? val.split('*')[0] : '',
  loadStellarSdk: async () => ({
    StrKey: {
      isValidEd25519PublicKey: (val) => val.startsWith('G') && val.length === 56,
    },
  }),
  resolveRecipient: async (val) => {
    if (val.startsWith('G') && val.length === 56) return { address: val };
    return { tag: val + '*localhost' };
  },
}));

vi.mock('react-hot-toast', () => {
  const mockToast = {
    loading: vi.fn(),
    success: vi.fn(),
    error: vi.fn(),
  };
  return {
    default: mockToast,
    toast: mockToast,
    Toaster: () => null,
  };
});

describe('Wallet Signing Failure and State Rollback (#291)', () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  const TEST_PUBLIC_KEY = 'GBRPYHIL2CI3FNQ4BXLFMNDLFJUNPU2HY3ZMFTGOBMAOTQTVHXB5TCYL';
  const RECIPIENT_KEY = 'GAAZI4TCR3TY5OJHCTJC2A4QSY6CJWJH5IAJTGKIN2ER7LBNVKOCCWN7';

  it('displays user-friendly error and rolls back state when transaction signing fails in Dashboard', async () => {
    const freighter = await import('@stellar/freighter-api');
    freighter.default.signTransaction.mockRejectedValue(new Error('User declined transaction'));

    render(
      <Dashboard
        userPublicKey={TEST_PUBLIC_KEY}
        isWalletConnected={true}
        balance={100}
        isBalanceRefreshing={false}
        onRefreshBalance={vi.fn()}
        onConnectWallet={vi.fn()}
        onDisconnectWallet={vi.fn()}
        onRegisterClick={vi.fn()}
        onAnalyticsClick={vi.fn()}
        onHistoryClick={vi.fn()}
        onHelpClick={vi.fn()}
        onRegistrationStateChange={vi.fn()}
        canRegister={true}
      />
    );

    // Enter recipient and amount
    const recipientInput = screen.getByPlaceholderText(/username or wallet address/i);
    const amountInput = screen.getByPlaceholderText(/0.00/i);
    const sendButton = screen.getByRole('button', { name: /Send Payment/i });

    fireEvent.change(recipientInput, { target: { value: RECIPIENT_KEY } });
    fireEvent.change(amountInput, { target: { value: '10' } });

    // Submit payment
    fireEvent.click(sendButton);

    // Wait for the explicit error message to appear
    await waitFor(() => {
      const errorMsg = screen.getByText(
        'Transaction signing was cancelled or failed — no transaction was submitted'
      );
      expect(errorMsg).toBeTruthy();
    });

    // Verify optimistic state is cleanly rolled back (button is not disabled/stuck)
    expect(sendButton.hasAttribute('disabled')).toBe(false);
  });

  it('displays user-friendly error and rolls back state when message signing fails in RegistrationPage', async () => {
    mockSignMessage.mockRejectedValue(new Error('Popup closed by user'));

    render(
      <RegistrationPage
        userPublicKey={TEST_PUBLIC_KEY}
        onRegistrationComplete={vi.fn()}
        onBackToDashboard={vi.fn()}
      />
    );

    const usernameInput = screen.getByPlaceholderText(/stellar-fan/i);
    const registerButton = screen.getByRole('button', { name: /Register username/i });

    fireEvent.change(usernameInput, { target: { value: 'alice' } });

    // Click register
    fireEvent.click(registerButton);

    // Verify error status message appears
    await waitFor(() => {
      const errorMsg = screen.getByText(
        'Transaction signing was cancelled or failed — no transaction was submitted'
      );
      expect(errorMsg).toBeTruthy();
    });

    // Verify state rolled back (button enabled for retry)
    expect(registerButton.hasAttribute('disabled')).toBe(false);
  });
});
