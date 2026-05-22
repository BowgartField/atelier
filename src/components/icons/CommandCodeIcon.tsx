import type { LucideProps } from 'lucide-react'
import { forwardRef } from 'react'

export const CommandCodeIcon = forwardRef<SVGSVGElement, LucideProps>(
  ({ size = 24, ...props }, ref) => (
    <svg
      ref={ref}
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 512 512"
      fill="none"
      stroke="currentColor"
      strokeWidth="42"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-label="Command Code"
      {...props}
    >
      <path d="M184 160 88 256l96 96" />
      <path d="M328 160l96 96-96 96" />
      <path d="m292 112-72 288" />
    </svg>
  )
)

CommandCodeIcon.displayName = 'CommandCodeIcon'
