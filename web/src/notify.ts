/**
 * Desktop notifications for the one thing that cannot wait for somebody to look at the window.
 *
 * An agent's request expires on its own after ninety seconds. Until now the only way to see it was
 * to have this window in front of you, which is exactly what somebody driving an assistant in a
 * terminal does not have. So the request also knocks on the desktop.
 *
 * Three deliberate limits:
 *
 *  - **Nothing secret is ever in a notification.** The record's title and the field's label are the
 *    most it carries — the same two things the dialog names before anybody decides anything. A
 *    notification is drawn by the desktop, may be logged by it, and can sit on a lock screen.
 *  - **Permission is asked from a click**, never on load. Browsers punish pages that ask cold, and
 *    a vault that opens with a permission prompt is a vault that taught you to dismiss its prompts.
 *  - **It only knocks when nobody is looking.** With the window focused the dialog is already on
 *    screen, and a notification for something you are staring at is noise.
 */

/** The icon the desktop shortcut uses, served by the binary itself. */
const ICON = '/icon-512.png'

export type NotifyState = NotificationPermission | 'unsupported'

export function notifyState(): NotifyState {
  return typeof Notification === 'undefined' ? 'unsupported' : Notification.permission
}

/**
 * Ask for permission. Call this from a click; browsers require it, and so does good manners.
 */
export async function askToNotify(): Promise<NotifyState> {
  if (typeof Notification === 'undefined') {
    return 'unsupported'
  }

  try {
    return await Notification.requestPermission()
  } catch {
    return Notification.permission
  }
}

/** Whether a knock would actually reach the desktop right now. */
export function canNotify(): boolean {
  return notifyState() === 'granted'
}

/**
 * Knock, unless the person is already looking at this window.
 *
 * `tag` collapses repeats: a request polled twice never becomes two notifications. Returns the
 * notification so the caller can take it back when the question is answered — a stale "approve
 * this" sitting in a notification tray after the request expired is worse than no notification.
 */
export function knock(options: {
  title: string
  body: string
  tag: string
  onClick?: () => void
}): Notification | null {
  if (!canNotify() || document.hasFocus()) {
    return null
  }

  try {
    const notification = new Notification(options.title, {
      body: options.body,
      icon: ICON,
      badge: ICON,
      tag: options.tag,
      // The question expires by itself; until then it should stay on screen rather than fade after
      // a few seconds into a tray nobody opens.
      requireInteraction: true,
    })

    notification.onclick = () => {
      window.focus()
      notification.close()
      options.onClick?.()
    }

    return notification
  } catch {
    // Some desktops refuse to construct one at all (no notification daemon, a headless session).
    // The dialog in the window is still there; this was only ever the second way to find out.
    return null
  }
}
