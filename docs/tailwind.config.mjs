/** @type {import('tailwindcss').Config} */
export default {
  content: ["./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}"],
  theme: {
    extend: {
      colors: {
        brand: {
          gold: "#eeaa00",
          "gold-dark": "#c89000",
        },
        dark: {
          950: "#0c1222", // Main background
          900: "#111827", // Card background
          800: "#1f2937", // Borders, subtle elements
          700: "#374151", // Hover states, borders
        },
        light: {
          100: "#f8fafc", // Primary text
          200: "#e2e8f0",
          300: "#cbd5e1",
          400: "#94a3b8", // Secondary text
        },
      },
      animation: {
        "fade-in-up": "fadeInUp 0.6s ease-out forwards",
        "subtle-glow": "subtleGlow 4s ease-in-out infinite",
      },
      keyframes: {
        fadeInUp: {
          "0%": { opacity: "0", transform: "translateY(20px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        subtleGlow: {
          "0%": { boxShadow: "0 0 20px -10px #eeaa00" },
          "50%": { boxShadow: "0 0 30px 0px #eeaa00" },
          "100%": { boxShadow: "0 0 20px -10px #eeaa00" },
        },
      },
    },
  },
  plugins: [],
};
