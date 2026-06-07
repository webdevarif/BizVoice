/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './settings.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: '#1a1a1f',
        panel: '#24242b',
        accent: '#3b82f6',
      },
    },
  },
  plugins: [],
};
