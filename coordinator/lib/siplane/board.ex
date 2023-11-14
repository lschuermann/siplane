defmodule Siplane.Board do
  use Ecto.Schema
  import Ecto.Changeset

  def log_event(event) do
    for board <- event.log_event_boards do
      Registry.lookup(Siplane.BoardOrchestrator.Registry, board.board_id)
      |> Enum.each(fn {pid, _} -> Siplane.BoardOrchestrator.log_event(pid, event) end)
    end
  end

  @primary_key {:id, :binary_id, autogenerate: true}
  @foreign_key_type :binary_id
  schema "boards" do
    field :label, :string
    field :location, :string
    field :manufacturer, :string
    field :hwrev, :string
    field :model, :string
    field :runner_token, :string
    field :image_url, :string

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

defmodule Siplane.Board.LogEvent do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key false
  @foreign_key_type :binary_id
  schema "log_event_boards" do
    belongs_to :log_event, Siplane.Log
    belongs_to :board, Siplane.Board

    field :public, :boolean
    field :owner_visible, :boolean
  end
end
