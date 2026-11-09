module.exports = {
  prefix: 'oe-',
  content: ['./apps/web/index.html', './apps/web/src/**/*.{ts,tsx}'],
  corePlugins: { preflight: false },
  theme: { extend: { colors: { background:'var(--background)', popover:'var(--popover)', 'popover-foreground':'var(--popover-foreground)', sidebar:'var(--sidebar)', 'sidebar-foreground':'var(--sidebar-foreground)', 'sidebar-accent':'var(--sidebar-accent)', 'sidebar-accent-foreground':'var(--sidebar-accent-foreground)', 'sidebar-border':'var(--sidebar-border)', 'sidebar-ring':'var(--sidebar-ring)', primary: 'var(--primary)', 'primary-foreground': 'var(--primary-foreground)', secondary: 'var(--secondary)', 'secondary-foreground': 'var(--secondary-foreground)', border: 'var(--border)', foreground: 'var(--foreground)', muted: 'var(--muted)', 'muted-foreground': 'var(--muted-foreground)', input: 'var(--input)', ring: 'var(--ring)', destructive: 'var(--destructive)' } } },
};
