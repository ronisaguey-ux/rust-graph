defmodule Scanner do
  @moduledoc "Walk a repository and summarise it."
  @depth 12

  def new(root, opts \\ []) do
    %{root: root, depth: Keyword.get(opts, :depth, @depth), seen: MapSet.new()}
  end

  def read(state, dir) do
    case File.ls(dir) do
      {:ok, entries} -> Enum.map(entries, fn entry -> Path.join(dir, entry) end)
      {:error, reason} -> IO.warn(reason)
    end
  end

  def scan(root) do
    state = new(root)
    files = read(state, root)
    bytes = files |> Enum.map(fn file -> File.stat!(file).size end) |> Enum.sum()
    %{count: length(files), bytes: bytes}
  end
end
