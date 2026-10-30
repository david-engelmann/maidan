// The console scaffold. Screens arrive one per change; the old board at /ui
// stays served until they all have.
export default function App() {
  return (
    <main className="splash">
      <div>
        <div className="mark steel" aria-hidden="true" />
        <h1>Maidan console</h1>
        <p>The new console is being built here. The board is still at /ui.</p>
      </div>
    </main>
  );
}
