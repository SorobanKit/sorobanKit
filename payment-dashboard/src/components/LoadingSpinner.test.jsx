// @vitest-environment jsdom
import { describe, it, expect, afterEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/react';
import LoadingSpinner from './LoadingSpinner';

describe('LoadingSpinner', () => {
  afterEach(() => {
    cleanup();
  });

  it('renders with role="status"', () => {
    render(<LoadingSpinner />);
    const spinner = screen.getByRole('status');
    expect(spinner).toBeDefined();
  });

  it('renders default label "Loading..." on aria-label and visually-hidden text', () => {
    render(<LoadingSpinner />);
    const spinner = screen.getByRole('status');
    expect(spinner.getAttribute('aria-label')).toBe('Loading...');
    expect(spinner.textContent).toContain('Loading...');
  });

  it('supports custom label prop', () => {
    render(<LoadingSpinner label="Fetching transactions..." />);
    const spinner = screen.getByRole('status');
    expect(spinner.getAttribute('aria-label')).toBe('Fetching transactions...');
    expect(spinner.textContent).toContain('Fetching transactions...');
  });

  it('renders svg with aria-hidden="true" to prevent redundant reader announcements', () => {
    const { container } = render(<LoadingSpinner size="w-8 h-8" color="text-nova" />);
    const svg = container.querySelector('svg');
    expect(svg).toBeDefined();
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.getAttribute('class')).toContain('w-8 h-8');
    expect(svg.getAttribute('class')).toContain('text-nova');
  });
});
