defmodule Siplane.Board do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "boards" do
    field :label, :string
    field :location, :string
    field :manufacturer, :string
    field :hwrev, :string
    field :model, :string
    field :runner_token, :string

    timestamps(type: :utc_datetime)
  end

  def runner_connected?(board_id) do
    Registry.lookup(Siplane.BoardOrchestrator.Registry, board_id)
    |> Enum.map(fn {pid, _} -> Siplane.BoardOrchestrator.runner_connected?(pid) end)
    |> List.first(false)
  end


  @doc false
  def changeset(board, attrs) do
    board
    |> cast(attrs, [:label, :manufacturer, :model, :hwrev, :location, :runner_token])
    |> validate_required([:label, :manufacturer, :model, :hwrev, :location, :runner_token])
  end
end
