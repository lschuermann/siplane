defmodule Siplane.Board.LogEvent do
  use Ecto.Schema
  # import Ecto.Changeset

  @primary_key false
  @foreign_key_type :binary_id
  schema "log_event_boards" do
    belongs_to :log_event, Siplane.Log
    belongs_to :board, Siplane.Board

    field :public, :boolean
    field :owner_visible, :boolean
  end
end
