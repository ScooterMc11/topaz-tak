import React from "react";
import ReactDOM from "react-dom/client";
import CssBaseline from "@mui/material/CssBaseline";
import { ThemeProvider, createTheme } from "@mui/material/styles";
import App from "./App";

// Inter across the whole app. The face is loaded via a <link> in index.html; here we make it the
// default family for every MUI component (Typography, inputs, buttons, etc.).
const INTER_STACK = '"Inter", "Roboto", "Helvetica", "Arial", sans-serif';
const theme = createTheme({
  palette: { mode: "dark" },
  typography: { fontFamily: INTER_STACK },
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App />
    </ThemeProvider>
  </React.StrictMode>
);
