// Replaces the page's `Notification` with one backed by the app's native notifications, on the
// board's origin only. The page then reports that it rings (X-Adjutant-Notify), so the server
// keeps quiet while the app is open and speaks again when it is closed.
(function () {
  const onBoard = location.protocol === 'http:'
    && ['127.0.0.1', 'localhost', '[::1]'].includes(location.hostname);
  if (!onBoard || !window.__TAURI_INTERNALS__) return;

  const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
  // Read synchronously by the page when it loads, so it is filled in by the app at window creation.
  let permission = '__PERMISSION__';

  // Notifications waiting for a click, by the number the app gave them (it counts for the whole app run,
  // so a click on one from an earlier page finds nothing here). Bounded: most are never clicked.
  const shown = new Map();
  window.__adjNotificationClicked = id => {
    const n = shown.get(id);
    if (n) n._fire('click');
  };

  const remember = value => {
    if (value === 'granted' || value === 'denied' || value === 'default') permission = value;
    return permission;
  };

  class AppNotification extends EventTarget {
    constructor(title, options = {}) {
      super();
      this.title = String(title);
      this.body = options.body == null ? '' : String(options.body);
      this.tag = options.tag == null ? '' : String(options.tag);
      this.icon = options.icon || '';
      this.data = options.data;
      this.onclick = null;
      this.onclose = null;
      this.onerror = null;
      this.onshow = null;
      if (permission !== 'granted') return;
      invoke('notify', { title: this.title, body: this.body }).then(
        id => {
          shown.set(id, this);
          if (shown.size > 50) shown.delete(shown.keys().next().value);
          this._fire('show');
        },
        () => this._fire('error'),
      );
    }

    // A native notification cannot be taken back once it is posted, so there is nothing to close.
    close() {}

    _fire(type) {
      const event = new Event(type);
      const handler = this['on' + type];
      if (typeof handler === 'function') handler.call(this, event);
      this.dispatchEvent(event);
    }

    static get permission() { return permission; }
    static get maxActions() { return 0; }

    static requestPermission(callback) {
      const asked = invoke('request_notification_permission').then(remember, () => permission);
      if (typeof callback === 'function') asked.then(callback);
      return asked;
    }
  }

  Object.defineProperty(window, 'Notification', { value: AppNotification, writable: true, configurable: true });

  // The system can change the answer while the app runs.
  invoke('notification_permission').then(remember, () => {});
})();
