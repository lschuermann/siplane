defmodule Siplane.BoardEnvironment do
  use Ecto.Schema

  @primary_key false
  @foreign_key_type :binary_id
  schema "board_environments" do
    belongs_to :board, Siplane.Board
    belongs_to :environment, Siplane.Environment

    timestamps(type: :utc_datetime)
  end
end
