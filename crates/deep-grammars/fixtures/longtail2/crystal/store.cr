require "yaml"

module Store
  alias Entry = NamedTuple(key: String, value: Int32)

  class Ledger
    @entries : Array(Entry)

    def initialize
      @entries = Array(Entry).new
    end

    def add(key : String, value : Int32) : Nil
      @entries << Entry.new(key: key, value: value)
    end

    def size : Int32
      @entries.size
    end

    private def reset : Nil
      @entries.clear
    end
  end
end

lib LibC
  fun getpid : Int32
end

ledger = Store::Ledger.new
ledger.add("alpha", 1)
total = ledger.size
while total > 0
  total -= 1
end
