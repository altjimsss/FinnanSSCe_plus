export default function App() {
  return (
    <main
      style={{
        minHeight: "100vh",
        display: "grid",
        placeItems: "center",
        margin: 0,
        fontFamily: "system-ui, sans-serif",
        background: "#f6f8f7",
        color: "#13211b",
      }}
    >
      <section
        style={{
          width: "min(720px, 92vw)",
          border: "1px solid #dbe4df",
          borderRadius: 14,
          padding: 24,
          background: "#ffffff",
        }}
      >
        <h1 style={{ margin: "0 0 8px" }}>FinnanSSCe Frontend Scaffold</h1>
        <p style={{ margin: 0, color: "#4d5f57", lineHeight: 1.5 }}>
          UI scaffold is ready. Contract integration can be added next.
        </p>
      </section>
    </main>
  );
}
