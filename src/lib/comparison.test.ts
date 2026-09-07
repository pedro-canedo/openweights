import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), acquire: vi.fn(), release: vi.fn(), unlisten: vi.fn() }));
vi.mock("./tauri", () => ({ isTauri: true, invoke: mocks.invoke, listen: mocks.listen }));
vi.mock("./generationStore", () => ({ generationStore: { acquireBenchmark: mocks.acquire } }));
beforeEach(() => {
  vi.resetModules(); vi.resetAllMocks();
  mocks.acquire.mockReturnValue(mocks.release);
  mocks.listen.mockResolvedValue(mocks.unlisten);
});

it("keeps measurement progress available after a subscriber leaves and returns", async () => {
  let finish!: (value: object) => void;
  mocks.invoke.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const { runComparison, comparisonOperation: state } = await import("./comparison");
  const unsubscribe = state.subscribe(vi.fn());
  const pending = runComparison("model");
  await Promise.resolve();
  unsubscribe();
  mocks.listen.mock.calls[0][1]({ model: "model", arm: 1, sample: 2 });
  expect(state.snapshot()).toMatchObject({ kind: "measure", progress: { arm: 1, sample: 2 } });
  expect(mocks.release).not.toHaveBeenCalled();
  const returned = vi.fn();
  const un = state.subscribe(returned);
  finish({ model: "model" });
  await pending;
  expect(state.snapshot().kind).toBeNull();
  expect(returned).toHaveBeenCalled();
  expect(mocks.release).toHaveBeenCalledOnce();
  expect(mocks.unlisten).toHaveBeenCalledOnce();
  un();
});

it("releases the queue and retains cancellation details on failure", async () => {
  mocks.invoke.mockRejectedValue(new Error("comparison-cancelled"));
  const { runComparison, comparisonOperation: state } = await import("./comparison");
  await expect(runComparison("model")).rejects.toThrow("comparison-cancelled");
  expect(state.snapshot()).toMatchObject({ kind: null, error: "Error: comparison-cancelled" });
  expect(mocks.release).toHaveBeenCalledOnce();
});

it("distinguishes applying from a cancellable measurement", async () => {
  let finish!: () => void;
  mocks.invoke.mockImplementation(() => new Promise<void>(resolve => { finish = resolve; }));
  const { applyComparison, comparisonOperation: state } = await import("./comparison");
  const pending = applyComparison("model", true);
  expect(state.snapshot().kind).toBe("apply");
  expect(mocks.invoke).toHaveBeenCalledWith("compare_apply", { model: "model", restore: true });
  finish(); await pending;
  expect(state.snapshot().kind).toBeNull();
  expect(mocks.release).toHaveBeenCalledOnce();
});
