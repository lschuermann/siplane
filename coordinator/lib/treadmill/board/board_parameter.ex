defmodule Treadmill.Board.Parameter do
  use Ecto.Schema

  @primary_key false
  @foreign_key_type :binary_id
  schema "board_parameters" do
    field :key, :string
    field :value, :string
    field :secret, :boolean

    belongs_to :board, Treadmill.Board

    timestamps(type: :utc_datetime)
  end
end
