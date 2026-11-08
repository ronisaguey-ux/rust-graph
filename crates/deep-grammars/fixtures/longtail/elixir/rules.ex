defmodule Scanner.Rules do
  defstruct(name: :size, max: 1_000_000)

  def build(name, max) do
    %__MODULE__{name: name, max: max}
  end

  def applies?(%__MODULE__{name: :size, max: max}, path) do
    File.stat!(path).size <= max
  end

  def applies?(%__MODULE__{name: :depth, max: max}, path) do
    length(Path.split(path)) <= max
  end

  def evaluate(rules, files) do
    Enum.filter(files, fn file ->
      Enum.all?(rules, fn rule -> applies?(rule, file) end)
    end)
  end
end
