defmodule TreadmillWeb.API.ErrorView do
  #use TreadmillWeb, :view

  def render("bad_request.json", assigns) do
    error = %{type: "bad_request"}

    if Map.has_key?(assigns, :description) do
      Map.put(error, :description, assigns.description)
    else
      error
    end
  end

  def render("not_found.json", _assigns) do
    %{
      type: "not_found",
      description: "The requested resource was not found."
    }
  end

  # By default, Phoenix returns the status message from
  # the template name. For example, "404.html" becomes
  # "Not Found".
  def template_not_found(template, _assigns) do
    Phoenix.Controller.status_message_from_template(template)
  end
end
