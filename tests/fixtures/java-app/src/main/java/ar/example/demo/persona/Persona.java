package ar.example.demo.persona;

public class Persona {
    private final String name;

    public Persona(String name) {
        this.name = name;
    }

    public void vote(String candidate) {
        System.out.println(name + " votes for " + candidate);
    }
}
