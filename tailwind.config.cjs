module.exports = {
  prefix: 'oe-',
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  corePlugins: { preflight: false },
  theme: { extend: { colors: { primary: 'var(--primary)', 'primary-foreground': 'var(--primary-foreground)', secondary: 'var(--secondary)', 'secondary-foreground': 'var(--secondary-foreground)', border: 'var(--border)', foreground: 'var(--foreground)', muted: 'var(--muted)', 'muted-foreground': 'var(--muted-foreground)', input: 'var(--input)', ring: 'var(--ring)', destructive: 'var(--destructive)' } } },
};
