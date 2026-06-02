import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import {
  TOAST_ACTION_SHORTCUT,
  ToastActionLabel,
} from '@/lib/toast-action-label'

describe('ToastActionLabel', () => {
  it('renders the action text with the universal toast action shortcut', () => {
    render(<ToastActionLabel>Resolve Conflicts</ToastActionLabel>)

    expect(screen.getByText('Resolve Conflicts')).toBeInTheDocument()
    expect(screen.getByText(TOAST_ACTION_SHORTCUT.label)).toBeInTheDocument()
  })
})
