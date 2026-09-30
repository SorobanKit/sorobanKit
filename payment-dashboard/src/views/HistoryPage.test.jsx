// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import HistoryPage from './HistoryPage';

vi.mock('@creit.tech/stellar-wallets-kit', () => {
  class Stub {}
  return {
    StellarWalletsKit: Stub,
    WalletNetwork: { TESTNET: 'TESTNET' },
    FreighterModule: Stub,
    xBullModule: Stub,
    AlbedoModule: Stub,
    LobstrModule: Stub,
  };
});

const PUBLIC_KEY = 'GABC1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890AB';
const noop = () => {};

const makeRecord = (id) => ({
  id: String(id),
  type: 'payment',
  from: 'GSENDER111',
  to: PUBLIC_KEY,
  amount: '10.0',
  asset_type: 'native',
  transaction_successful: true,
  created_at: new Date(2026, 0, id).toISOString(),
  transaction_hash: `hash${id}`,
});

const makeHorizonResponse = (records, nextCursor = null) => ({
  _embedded: { records },
  _links: {
    next: nextCursor
      ? { href: `https://horizon-testnet.stellar.org/accounts/x/payments?cursor=${nextCursor}&order=desc&limit=20` }
      : { href: '' },
  },
});

const renderPage = (key = PUBLIC_KEY) =>
  render(
    <HistoryPage
      userPublicKey={key}
      setUserPublicKey={noop}
      onConnectWallet={noop}
      onDisconnectWallet={noop}
      onRefreshBalance={noop}
      onDashboardClick={noop}
      onAnalyticsClick={noop}
      onHelpClick={noop}
      onRegisterClick={noop}
      canRegister={false}
    />,
  );

describe('HistoryPage pagination', () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('fetches only the first page (limit=20) on initial render', async () => {
    const firstPageRecords = Array.from({ length: 20 }, (_, i) => makeRecord(i + 1));
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => makeHorizonResponse(firstPageRecords, 'cursor-page-2'),
    });
    vi.stubGlobal('fetch', fetchMock);
    vi.stubGlobal('matchMedia', () => ({ matches: false, addListener: noop, removeListener: noop }));

    renderPage();

    await waitFor(() => expect(screen.queryByText('Loading transactions...')).toBeNull());

    const [url] = fetchMock.mock.calls[0];
    expect(url).toContain('limit=20');
    expect(url).not.toContain('cursor=');
  });

  it('shows a "Load more" button when the API returns a next cursor', async () => {
    const firstPageRecords = Array.from({ length: 20 }, (_, i) => makeRecord(i + 1));
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => makeHorizonResponse(firstPageRecords, 'cursor-page-2'),
    }));
    vi.stubGlobal('matchMedia', () => ({ matches: false, addListener: noop, removeListener: noop }));

    renderPage();

    const loadMoreBtn = await screen.findByRole('button', { name: /load more/i });
    expect(loadMoreBtn).toBeTruthy();
  });

  it('does not show "Load more" when there is no next page', async () => {
    const records = Array.from({ length: 5 }, (_, i) => makeRecord(i + 1));
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => makeHorizonResponse(records, null),
    }));
    vi.stubGlobal('matchMedia', () => ({ matches: false, addListener: noop, removeListener: noop }));

    renderPage();

    await waitFor(() => expect(screen.queryByText('Loading transactions...')).toBeNull());
    expect(screen.queryByRole('button', { name: /load more/i })).toBeNull();
  });

  it('fetches the next page with the cursor when "Load more" is clicked', async () => {
    const page1 = Array.from({ length: 20 }, (_, i) => makeRecord(i + 1));
    const page2 = Array.from({ length: 5 }, (_, i) => makeRecord(i + 21));
    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => makeHorizonResponse(page1, 'cursor-page-2'),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: async () => makeHorizonResponse(page2, null),
      });
    vi.stubGlobal('fetch', fetchMock);
    vi.stubGlobal('matchMedia', () => ({ matches: false, addListener: noop, removeListener: noop }));

    renderPage();

    const loadMoreBtn = await screen.findByRole('button', { name: /load more/i });
    fireEvent.click(loadMoreBtn);

    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

    const [secondUrl] = fetchMock.mock.calls[1];
    expect(secondUrl).toContain('cursor=cursor-page-2');
    expect(secondUrl).toContain('limit=20');
  });

  it('shows a loading spinner while fetching more records', async () => {
    let resolvePage2;
    const page2Promise = new Promise((resolve) => { resolvePage2 = resolve; });
    const page1 = Array.from({ length: 20 }, (_, i) => makeRecord(i + 1));

    const fetchMock = vi.fn()
      .mockResolvedValueOnce({
        ok: true,
        json: async () => makeHorizonResponse(page1, 'cursor-p2'),
      })
      .mockReturnValueOnce(page2Promise);
    vi.stubGlobal('fetch', fetchMock);
    vi.stubGlobal('matchMedia', () => ({ matches: false, addListener: noop, removeListener: noop }));

    renderPage();

    const loadMoreBtn = await screen.findByRole('button', { name: /load more/i });
    fireEvent.click(loadMoreBtn);

    await waitFor(() => expect(screen.queryByText('Loading more...')).not.toBeNull());

    resolvePage2({ ok: true, json: async () => makeHorizonResponse([], null) });
    await waitFor(() => expect(screen.queryByText('Loading more...')).toBeNull());
  });
});
