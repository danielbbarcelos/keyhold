import { useId } from 'react'

import { cn } from '@/lib/utils'

/**
 * The AWS mark Keyhold uses wherever an AWS account or session appears: the Systems Manager
 * drawing (a cloud and a gear with three sliders) on its pink gradient, in a circle.
 *
 * Drawn here rather than loaded: the page may only show what the program carries.
 */
const SIZES = {
  xs: 'size-[22px]',
  sm: 'size-8',
  md: 'size-9',
  lg: 'size-12',
}

/** A gear with eight teeth around (cx, cy), as one closed path. */
function gear(cx: number, cy: number, outer: number, inner: number, teeth = 8): string {
  const points: string[] = []
  const step = (Math.PI * 2) / teeth
  const half = step / 4.4

  for (let i = 0; i < teeth; i += 1) {
    const a = i * step - Math.PI / 2
    const at = (angle: number, r: number) =>
      `${(cx + Math.cos(angle) * r).toFixed(2)} ${(cy + Math.sin(angle) * r).toFixed(2)}`

    points.push(at(a - half * 1.5, inner), at(a - half, outer), at(a + half, outer), at(a + half * 1.5, inner))
  }

  return `M${points.join(' L')} Z`
}

const GEAR = gear(41, 40, 15.5, 12)

export function AwsMark({
  size = 'sm',
  className,
  title,
}: {
  size?: keyof typeof SIZES
  className?: string
  title?: string
}) {
  const id = useId().replace(/:/g, '')
  const fill = `aws-${id}`

  return (
    <svg
      viewBox="0 0 64 64"
      role={title ? 'img' : undefined}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      className={cn('shrink-0 rounded-full', SIZES[size], className)}
    >
      <defs>
        <linearGradient id={fill} x1="0" y1="64" x2="64" y2="0" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#B0084D" />
          <stop offset="1" stopColor="#FF4F8B" />
        </linearGradient>
      </defs>
      <circle cx="32" cy="32" r="32" fill={`url(#${fill})`} />
      <g fill="none" stroke="#fff" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
        <path d="M27 40 H17.5 C13.4 40 10.5 37 10.5 33.2 C10.5 29.6 13.2 26.7 16.8 26.4 C17.6 20.4 22.4 16 28.4 16 C33 16 36.9 18.6 38.7 22.4 C39.8 21.9 41 21.6 42.3 21.6 C46.8 21.6 50.4 24.9 50.9 29.2" />
        <path d={GEAR} fill={`url(#${fill})`} />
        <path d="M34.5 35.5 H47.5 M34.5 40 H47.5 M34.5 44.5 H47.5" strokeWidth="2.2" />
        <path d="M44.5 33.5 V37.5 M39.5 38 V42 M37 42.5 V46.5" strokeWidth="2.2" />
      </g>
    </svg>
  )
}
