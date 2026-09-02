import { Outlet, createRootRoute } from "@tanstack/react-router";

export const Route = createRootRoute({
  component: RootLayout,
  notFoundComponent: () => (
    <section className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
      <h1 className="text-2xl font-semibold">Page not found</h1>
      <a className="text-sm underline" href="/">
        Open repositories
      </a>
    </section>
  ),
});

function RootLayout() {
  return (
    <div className="flex min-h-dvh flex-col">
      <main className="flex flex-1 flex-col">
        <Outlet />
      </main>
    </div>
  );
}
