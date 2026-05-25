package ar.example.demo.acreta.persona;
// @expancode-source: specs/persona/voting.yaml
import ar.example.demo.persona.Persona;
class Ref {
  static {
    Persona persona = new Persona("Alice");
    persona.vote("candidate-a");
  }
}